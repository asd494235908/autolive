use std::fmt;

/// mpv 在 Windows 上把 `--wid` 解析为 `uint32_t`，因此调用方必须使用这个窄类型，
/// 不能把 Rust 指针宽度的 HWND 十进制值直接拼进命令行。
pub type MpvWindowId = u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MpvVideoWindowInfo {
    pub window_id: MpvWindowId,
    pub parent_window_id: MpvWindowId,
    pub direct_parent_is_host: bool,
    pub host_is_ancestor: bool,
    pub visible: bool,
    pub client_width: u32,
    pub client_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MpvVideoWindowInspection {
    pub host_window_id: MpvWindowId,
    pub mpv_process_id: u32,
    pub enumerated_descendants: u32,
    pub process_owned_descendants: u32,
    pub visible_process_descendants: u32,
    pub non_empty_process_descendants: u32,
    pub video_window: Option<MpvVideoWindowInfo>,
}

impl MpvVideoWindowInspection {
    pub fn is_ready(&self) -> bool {
        self.video_window.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeVideoHostError {
    InvalidParent,
    NotReady,
    Destroyed,
    WrongThread,
    Unsupported,
    Platform(String),
}

impl fmt::Display for NativeVideoHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParent => formatter.write_str("最终效果父窗口句柄无效"),
            Self::NotReady => formatter.write_str("专用视频宿主窗口尚未就绪"),
            Self::Destroyed => formatter.write_str("专用视频宿主窗口已销毁"),
            Self::WrongThread => formatter.write_str("专用视频宿主窗口操作不在创建线程"),
            Self::Unsupported => formatter.write_str("专用视频宿主窗口仅支持 Windows"),
            Self::Platform(message) => write!(formatter, "Win32 视频宿主操作失败：{message}"),
        }
    }
}

impl std::error::Error for NativeVideoHostError {}

#[derive(Debug)]
pub struct NativeVideoHost {
    window_id: u64,
    parent_window_id: u64,
    owner_thread_id: u32,
}

impl NativeVideoHost {
    #[cfg(windows)]
    pub fn create(parent_window_id: u64) -> Result<Self, NativeVideoHostError> {
        windows_impl::create(parent_window_id)
    }

    #[cfg(not(windows))]
    pub fn create(_parent_window_id: u64) -> Result<Self, NativeVideoHostError> {
        Err(NativeVideoHostError::Unsupported)
    }

    #[cfg(windows)]
    pub fn ready_window_id(&self) -> Result<u64, NativeVideoHostError> {
        windows_impl::ready_window_id(self)
    }

    #[cfg(not(windows))]
    pub fn ready_window_id(&self) -> Result<u64, NativeVideoHostError> {
        Err(NativeVideoHostError::Unsupported)
    }

    /// 返回可直接传给 mpv `--wid` 的 Windows `uint32_t` 句柄值。
    #[cfg(windows)]
    pub fn mpv_wid(&self) -> Result<MpvWindowId, NativeVideoHostError> {
        windows_impl::mpv_wid(self)
    }

    #[cfg(not(windows))]
    pub fn mpv_wid(&self) -> Result<MpvWindowId, NativeVideoHostError> {
        Err(NativeVideoHostError::Unsupported)
    }

    /// 只读检查 mpv 是否已在本宿主下创建可见、非零客户区的视频子窗口。
    #[cfg(windows)]
    pub fn inspect_mpv_video_window(
        &self,
        mpv_process_id: u32,
    ) -> Result<MpvVideoWindowInspection, NativeVideoHostError> {
        windows_impl::inspect_mpv_video_window(self, mpv_process_id)
    }

    #[cfg(not(windows))]
    pub fn inspect_mpv_video_window(
        &self,
        _mpv_process_id: u32,
    ) -> Result<MpvVideoWindowInspection, NativeVideoHostError> {
        Err(NativeVideoHostError::Unsupported)
    }

    pub fn parent_window_id(&self) -> u64 {
        self.parent_window_id
    }

    #[cfg(windows)]
    pub fn resize_to_parent_client(&self) -> Result<(), NativeVideoHostError> {
        windows_impl::resize_to_parent_client(self)
    }

    #[cfg(not(windows))]
    pub fn resize_to_parent_client(&self) -> Result<(), NativeVideoHostError> {
        Err(NativeVideoHostError::Unsupported)
    }

    #[cfg(windows)]
    pub fn promote_to_top(&self) -> Result<(), NativeVideoHostError> {
        windows_impl::promote_to_top(self)
    }

    #[cfg(not(windows))]
    pub fn promote_to_top(&self) -> Result<(), NativeVideoHostError> {
        Err(NativeVideoHostError::Unsupported)
    }

    #[cfg(windows)]
    pub fn destroy(&mut self) -> Result<(), NativeVideoHostError> {
        windows_impl::destroy(self)
    }

    #[cfg(not(windows))]
    pub fn destroy(&mut self) -> Result<(), NativeVideoHostError> {
        Err(NativeVideoHostError::Unsupported)
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::{
        MpvVideoWindowInfo, MpvVideoWindowInspection, MpvWindowId, NativeVideoHost,
        NativeVideoHostError,
    };
    use std::sync::OnceLock;
    use windows::core::{w, Error as WindowsError, BOOL, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, EnumChildWindows, GetAncestor,
        GetClientRect, GetParent, GetWindowThreadProcessId, IsWindow, IsWindowVisible, MoveWindow,
        RegisterClassW, SetWindowPos, GA_PARENT, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_SHOWWINDOW, WINDOW_EX_STYLE, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
        WS_VISIBLE,
    };

    const CLASS_NAME: PCWSTR = w!("GpAutoLiveNativeVideoHost");
    static CLASS_REGISTRATION: OnceLock<Result<(), String>> = OnceLock::new();

    pub(super) fn create(parent_window_id: u64) -> Result<NativeVideoHost, NativeVideoHostError> {
        let parent = hwnd(parent_window_id).ok_or(NativeVideoHostError::InvalidParent)?;
        // SAFETY: parent 只由非零整数构造；IsWindow 不解引用调用方内存。
        if !unsafe { IsWindow(Some(parent)).as_bool() } {
            return Err(NativeVideoHostError::InvalidParent);
        }
        register_window_class()?;
        // SAFETY: GetCurrentThreadId 无参数且无资源所有权转移。
        let owner_thread_id = unsafe { GetCurrentThreadId() };
        let instance = module_instance()?;
        let rect = client_rect(parent)?;
        let width = rect.right.saturating_sub(rect.left);
        let height = rect.bottom.saturating_sub(rect.top);
        if width <= 0 || height <= 0 {
            return Err(NativeVideoHostError::NotReady);
        }
        // SAFETY: 已注册窗口类，parent/instance 有效，尺寸来自父窗口客户区；
        // lpParam 为空且窗口过程只转发给 DefWindowProcW。
        let child = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                CLASS_NAME,
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
                0,
                0,
                width,
                height,
                Some(parent),
                None,
                Some(instance),
                None,
            )
        }
        .map_err(platform_error)?;
        let mut host = NativeVideoHost {
            window_id: child.0 as usize as u64,
            parent_window_id,
            owner_thread_id,
        };
        if let Err(error) = ready_window_id(&host) {
            let _ignored = destroy(&mut host);
            return Err(error);
        }
        Ok(host)
    }

    pub(super) fn ready_window_id(host: &NativeVideoHost) -> Result<u64, NativeVideoHostError> {
        let child = hwnd(host.window_id).ok_or(NativeVideoHostError::Destroyed)?;
        // SAFETY: child 是本模块创建并按值保存的 HWND；IsWindow 负责验证其当前有效性。
        if !unsafe { IsWindow(Some(child)).as_bool() } {
            return Err(NativeVideoHostError::Destroyed);
        }
        let parent = hwnd(host.parent_window_id).ok_or(NativeVideoHostError::InvalidParent)?;
        // SAFETY: parent 是创建时保存的 HWND；IsWindow 负责验证其当前有效性。
        if !unsafe { IsWindow(Some(parent)).as_bool() } {
            return Err(NativeVideoHostError::InvalidParent);
        }
        // SAFETY: child 已由 IsWindow 验证有效；查询父窗口和可见性不转移所有权。
        if unsafe { GetParent(child) } != Ok(parent) || !unsafe { IsWindowVisible(child).as_bool() }
        {
            return Err(NativeVideoHostError::NotReady);
        }
        let rect = client_rect(child)?;
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return Err(NativeVideoHostError::NotReady);
        }
        Ok(host.window_id)
    }

    pub(super) fn mpv_wid(host: &NativeVideoHost) -> Result<MpvWindowId, NativeVideoHostError> {
        ready_window_id(host)?;
        let child = hwnd(host.window_id).ok_or(NativeVideoHostError::Destroyed)?;
        Ok(hwnd_to_mpv_wid(child))
    }

    pub(super) fn inspect_mpv_video_window(
        host: &NativeVideoHost,
        mpv_process_id: u32,
    ) -> Result<MpvVideoWindowInspection, NativeVideoHostError> {
        if mpv_process_id == 0 {
            return Err(NativeVideoHostError::Platform(
                "mpv 进程 PID 必须大于 0".to_owned(),
            ));
        }
        ready_window_id(host)?;
        let host_window = hwnd(host.window_id).ok_or(NativeVideoHostError::Destroyed)?;
        let mut state = EnumerationState {
            host_window,
            report: MpvVideoWindowInspection {
                host_window_id: hwnd_to_mpv_wid(host_window),
                mpv_process_id,
                enumerated_descendants: 0,
                process_owned_descendants: 0,
                visible_process_descendants: 0,
                non_empty_process_descendants: 0,
                video_window: None,
            },
        };
        // SAFETY: state 在同步枚举结束前保持有效；回调只读取窗口状态并更新该局部值。
        unsafe {
            EnumChildWindows(
                Some(host_window),
                Some(enumerate_mpv_window),
                LPARAM((&mut state as *mut EnumerationState) as isize),
            )
        }
        .ok()
        .map_err(platform_error)?;
        Ok(state.report)
    }

    pub(super) fn resize_to_parent_client(
        host: &NativeVideoHost,
    ) -> Result<(), NativeVideoHostError> {
        ensure_owner_thread(host)?;
        ready_window_id(host)?;
        let child = hwnd(host.window_id).ok_or(NativeVideoHostError::Destroyed)?;
        let parent = hwnd(host.parent_window_id).ok_or(NativeVideoHostError::InvalidParent)?;
        let rect = client_rect(parent)?;
        let width = rect.right.saturating_sub(rect.left);
        let height = rect.bottom.saturating_sub(rect.top);
        if width <= 0 || height <= 0 {
            return Err(NativeVideoHostError::NotReady);
        }
        // SAFETY: child 已验证且当前线程等于创建线程，尺寸来自有效父客户区。
        unsafe { MoveWindow(child, 0, 0, width, height, true) }.map_err(platform_error)
    }

    pub(super) fn promote_to_top(host: &NativeVideoHost) -> Result<(), NativeVideoHostError> {
        ensure_owner_thread(host)?;
        ready_window_id(host)?;
        let child = hwnd(host.window_id).ok_or(NativeVideoHostError::Destroyed)?;
        // SAFETY: child 已验证且当前线程等于创建线程；标志明确禁止改变位置、尺寸和激活状态。
        unsafe {
            SetWindowPos(
                child,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }
        .map_err(platform_error)
    }

    pub(super) fn destroy(host: &mut NativeVideoHost) -> Result<(), NativeVideoHostError> {
        ensure_owner_thread(host)?;
        let child = hwnd(host.window_id).ok_or(NativeVideoHostError::Destroyed)?;
        // SAFETY: child 是本模块保存的 HWND；IsWindow 验证销毁前是否仍有效。
        if !unsafe { IsWindow(Some(child)).as_bool() } {
            host.window_id = 0;
            return Err(NativeVideoHostError::Destroyed);
        }
        // SAFETY: child 有效且当前线程等于创建线程；销毁后立即清零，禁止再次使用。
        unsafe { DestroyWindow(child) }.map_err(platform_error)?;
        host.window_id = 0;
        Ok(())
    }

    fn register_window_class() -> Result<(), NativeVideoHostError> {
        CLASS_REGISTRATION
            .get_or_init(|| {
                let instance = module_instance().map_err(|error| error.to_string())?;
                let class = WNDCLASSW {
                    lpfnWndProc: Some(window_proc),
                    hInstance: instance,
                    lpszClassName: CLASS_NAME,
                    ..Default::default()
                };
                // SAFETY: WNDCLASSW 中的类名和窗口过程均为进程生命周期内有效的静态值。
                let atom = unsafe { RegisterClassW(&class) };
                if atom == 0 {
                    Err(WindowsError::from_win32().to_string())
                } else {
                    Ok(())
                }
            })
            .clone()
            .map_err(NativeVideoHostError::Platform)
    }

    fn module_instance() -> Result<HINSTANCE, NativeVideoHostError> {
        // SAFETY: None 请求当前进程模块，不取得需要释放的额外所有权。
        let module = unsafe { GetModuleHandleW(None) }.map_err(platform_error)?;
        Ok(HINSTANCE(module.0))
    }

    fn client_rect(window: HWND) -> Result<RECT, NativeVideoHostError> {
        let mut rect = RECT::default();
        // SAFETY: rect 是有效可写出参，调用方已验证 window 的有效性。
        unsafe { GetClientRect(window, &mut rect) }.map_err(platform_error)?;
        Ok(rect)
    }

    fn ensure_owner_thread(host: &NativeVideoHost) -> Result<(), NativeVideoHostError> {
        // SAFETY: GetCurrentThreadId 无参数且无资源所有权转移。
        if unsafe { GetCurrentThreadId() } == host.owner_thread_id {
            Ok(())
        } else {
            Err(NativeVideoHostError::WrongThread)
        }
    }

    fn hwnd(window_id: u64) -> Option<HWND> {
        usize::try_from(window_id)
            .ok()
            .filter(|value| *value != 0)
            .map(|value| HWND(value as *mut _))
    }

    fn hwnd_to_mpv_wid(window: HWND) -> MpvWindowId {
        window.0 as usize as MpvWindowId
    }

    fn ancestor_reaches_host(mut window: HWND, host: HWND) -> bool {
        for _ in 0..64 {
            // SAFETY: window 来自 EnumChildWindows；GA_PARENT 只查询当前父级，不转移所有权。
            let parent = unsafe { GetAncestor(window, GA_PARENT) };
            if parent == host {
                return true;
            }
            if parent.0.is_null() || parent == window {
                return false;
            }
            window = parent;
        }
        false
    }

    struct EnumerationState {
        host_window: HWND,
        report: MpvVideoWindowInspection,
    }

    unsafe extern "system" fn enumerate_mpv_window(window: HWND, state: LPARAM) -> BOOL {
        // SAFETY: inspect_mpv_video_window 传入局部 EnumerationState 的唯一可变指针，
        // EnumChildWindows 在返回前同步完成全部回调。
        let state = unsafe { &mut *(state.0 as *mut EnumerationState) };
        state.report.enumerated_descendants = state.report.enumerated_descendants.saturating_add(1);

        let mut process_id = 0;
        // SAFETY: process_id 是有效可写出参，window 由 Windows 枚举器提供。
        unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
        if process_id != state.report.mpv_process_id {
            return true.into();
        }
        state.report.process_owned_descendants =
            state.report.process_owned_descendants.saturating_add(1);

        // SAFETY: window 由 Windows 枚举器提供；可见性查询不转移所有权。
        let visible = unsafe { IsWindowVisible(window).as_bool() };
        if visible {
            state.report.visible_process_descendants =
                state.report.visible_process_descendants.saturating_add(1);
        }
        let rect = client_rect(window).ok();
        let width = rect
            .as_ref()
            .map(|rect| rect.right.saturating_sub(rect.left))
            .unwrap_or_default();
        let height = rect
            .as_ref()
            .map(|rect| rect.bottom.saturating_sub(rect.top))
            .unwrap_or_default();
        if width > 0 && height > 0 {
            state.report.non_empty_process_descendants =
                state.report.non_empty_process_descendants.saturating_add(1);
        }

        // SAFETY: window 由 Windows 枚举器提供；GetParent 只查询关系。
        let direct_parent = unsafe { GetParent(window) }.ok();
        let direct_parent_is_host = direct_parent == Some(state.host_window);
        let host_is_ancestor = ancestor_reaches_host(window, state.host_window);
        if state.report.video_window.is_none()
            && visible
            && width > 0
            && height > 0
            && host_is_ancestor
        {
            state.report.video_window = Some(MpvVideoWindowInfo {
                window_id: hwnd_to_mpv_wid(window),
                parent_window_id: direct_parent.map(hwnd_to_mpv_wid).unwrap_or_default(),
                direct_parent_is_host,
                host_is_ancestor,
                visible,
                client_width: width as u32,
                client_height: height as u32,
            });
        }
        true.into()
    }

    fn platform_error(error: WindowsError) -> NativeVideoHostError {
        NativeVideoHostError::Platform(error.to_string())
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: 参数由 Windows 传入，默认窗口过程是未处理消息的规定转发目标。
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    #[cfg(test)]
    mod tests {
        use super::{hwnd_to_mpv_wid, HWND};

        #[test]
        fn mpv_wid_uses_the_low_uint32_handle_value() {
            let window = HWND(0xffff_ffff_89ab_cdefusize as *mut _);
            assert_eq!(hwnd_to_mpv_wid(window), 0x89ab_cdef);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MpvVideoWindowInspection, NativeVideoHost, NativeVideoHostError};

    #[test]
    fn zero_parent_handle_is_rejected_before_win32_creation() {
        assert!(matches!(
            NativeVideoHost::create(0),
            Err(NativeVideoHostError::InvalidParent) | Err(NativeVideoHostError::Unsupported)
        ));
    }

    #[test]
    fn readiness_errors_are_distinguishable() {
        assert_ne!(
            NativeVideoHostError::NotReady.to_string(),
            NativeVideoHostError::Destroyed.to_string()
        );
    }

    #[test]
    fn inspection_is_ready_only_after_a_qualified_child_was_found() {
        let mut inspection = MpvVideoWindowInspection {
            host_window_id: 1,
            mpv_process_id: 2,
            enumerated_descendants: 1,
            process_owned_descendants: 0,
            visible_process_descendants: 0,
            non_empty_process_descendants: 0,
            video_window: None,
        };
        assert!(!inspection.is_ready());
        inspection.video_window = Some(super::MpvVideoWindowInfo {
            window_id: 3,
            parent_window_id: 1,
            direct_parent_is_host: true,
            host_is_ancestor: true,
            visible: true,
            client_width: 1280,
            client_height: 720,
        });
        assert!(inspection.is_ready());
    }
}
