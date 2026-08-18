const MAX_VIDEO_DIMENSION: u32 = 16_384;
const MIN_WINDOW_WIDTH: f64 = 320.0;
const MIN_WINDOW_HEIGHT: f64 = 180.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSizingError {
    InvalidVideoDimensions,
    InvalidWorkArea,
}

pub fn calculate_window_size(
    video_width: u32,
    video_height: u32,
    work_area_width: f64,
    work_area_height: f64,
    native_titlebar_height: f64,
) -> Result<WindowSize, WindowSizingError> {
    if video_width == 0
        || video_height == 0
        || video_width > MAX_VIDEO_DIMENSION
        || video_height > MAX_VIDEO_DIMENSION
    {
        return Err(WindowSizingError::InvalidVideoDimensions);
    }
    if !work_area_width.is_finite()
        || !work_area_height.is_finite()
        || !native_titlebar_height.is_finite()
        || work_area_width <= 0.0
        || work_area_height <= 0.0
        || native_titlebar_height < 0.0
    {
        return Err(WindowSizingError::InvalidWorkArea);
    }

    // titlebar 只占工作区，不计入 inner_size（Tauri set_size = 客户区）
    let available_video_height = (work_area_height - native_titlebar_height).max(1.0);
    let scale = (work_area_width / f64::from(video_width))
        .min(available_video_height / f64::from(video_height))
        .min(1.0);
    if !scale.is_finite() || scale <= 0.0 {
        return Err(WindowSizingError::InvalidWorkArea);
    }

    let mut width = round_to_dimension(f64::from(video_width) * scale);
    let mut height = round_to_dimension(f64::from(video_height) * scale);
    let max_width = round_to_dimension(work_area_width);
    let max_height = round_to_dimension(available_video_height);

    width = width.max(MIN_WINDOW_WIDTH as u32).min(max_width);
    height = height.max(MIN_WINDOW_HEIGHT as u32).min(max_height);

    Ok(WindowSize { width, height })
}

fn round_to_dimension(value: f64) -> u32 {
    value.round().clamp(1.0, u32::MAX as f64) as u32
}

#[cfg(test)]
mod tests {
    use super::{calculate_window_size, WindowSize, WindowSizingError};

    #[test]
    fn does_not_reserve_hidden_player_chrome_for_video_only_layout() {
        assert_eq!(
            calculate_window_size(1280, 720, 1920.0, 1200.0, 0.0),
            Ok(WindowSize {
                width: 1280,
                height: 720,
            })
        );
    }

    #[test]
    fn scales_large_video_without_changing_video_ratio() {
        assert_eq!(
            calculate_window_size(3840, 2160, 1440.0, 900.0, 32.0),
            Ok(WindowSize {
                width: 1440,
                height: 810,
            })
        );
    }

    #[test]
    fn accounts_for_native_titlebar_before_sizing_video() {
        assert_eq!(
            calculate_window_size(720, 1280, 1440.0, 1021.0, 32.0),
            Ok(WindowSize {
                width: 556,
                height: 989,
            })
        );
    }

    #[test]
    fn portrait_video_window_matches_scaled_video_without_side_bars() {
        assert_eq!(
            calculate_window_size(720, 1280, 1440.0, 900.0, 32.0),
            Ok(WindowSize {
                width: 488,
                height: 868,
            })
        );
    }

    #[test]
    fn applies_minimum_window_size_for_small_video() {
        assert_eq!(
            calculate_window_size(320, 180, 1440.0, 900.0, 32.0),
            Ok(WindowSize {
                width: 320,
                height: 180,
            })
        );
    }

    #[test]
    fn stays_inside_tiny_work_area_when_minimum_does_not_fit() {
        assert_eq!(
            calculate_window_size(1280, 720, 500.0, 300.0, 32.0),
            Ok(WindowSize {
                width: 476,
                height: 268,
            })
        );
    }

    #[test]
    fn rejects_missing_or_unsafe_video_dimensions() {
        assert_eq!(
            calculate_window_size(0, 720, 1920.0, 1200.0, 0.0),
            Err(WindowSizingError::InvalidVideoDimensions)
        );
        assert_eq!(
            calculate_window_size(16_385, 720, 1920.0, 1200.0, 0.0),
            Err(WindowSizingError::InvalidVideoDimensions)
        );
    }
}
