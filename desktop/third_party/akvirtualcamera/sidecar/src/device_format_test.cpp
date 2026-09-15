#include <windows.h>
#include <cstdio>
#include <cassert>
#include "device_format.h"

// Process-local HKLM override exercises the real Win32 boundary without elevation
// or modifying any installed camera. The disposable fixture lives under HKCU.
int main() {
    const auto testRoot = L"Software\\GpAutoLive\\Tests\\AkVirtualCamera-" + std::to_wstring(GetCurrentProcessId());
    HKEY fixture{};
    assert(RegCreateKeyExW(HKEY_CURRENT_USER, testRoot.c_str(), 0, nullptr, 0, KEY_ALL_ACCESS, nullptr, &fixture, nullptr) == ERROR_SUCCESS);
    assert(RegOverridePredefKey(HKEY_LOCAL_MACHINE, fixture) == ERROR_SUCCESS);
    auto key = [](const wchar_t *path) {
        HKEY result{};
        assert(RegCreateKeyExW(HKEY_LOCAL_MACHINE, path, 0, nullptr, 0, KEY_ALL_ACCESS | KEY_WOW64_64KEY, nullptr, &result, nullptr) == ERROR_SUCCESS);
        return result;
    };
    auto text = [](HKEY target, const wchar_t *name, const wchar_t *value) {
        assert(RegSetValueExW(target, name, 0, REG_SZ, reinterpret_cast<const BYTE *>(value), static_cast<DWORD>((wcslen(value) + 1) * sizeof(wchar_t))) == ERROR_SUCCESS);
    };
    auto number = [](HKEY target, const wchar_t *name, DWORD value) {
        assert(RegSetValueExW(target, name, 0, REG_DWORD, reinterpret_cast<const BYTE *>(&value), sizeof(value)) == ERROR_SUCCESS);
    };
    wchar_t module[32768]{}; GetModuleFileNameW(nullptr, module, 32768);
    std::wstring root(module);
    for (int index = 0; index < 2; ++index) root.resize(root.find_last_of(L"\\/"));
    auto owner = key(L"SOFTWARE\\Webcamoid\\VirtualCamera"); text(owner, L"installPath", root.c_str()); RegCloseKey(owner);
    auto cameras = key(L"SOFTWARE\\Webcamoid\\VirtualCamera\\Cameras"); number(cameras, L"size", 1); RegCloseKey(cameras);
    auto camera = key(L"SOFTWARE\\Webcamoid\\VirtualCamera\\Cameras\\1");
    text(camera, L"id", L"GpAutoLiveCamera"); text(camera, L"description", L"GpAutoLive Camera"); RegCloseKey(camera);
    auto formats = key(L"SOFTWARE\\Webcamoid\\VirtualCamera\\Cameras\\1\\Formats"); number(formats, L"size", 1); RegCloseKey(formats);
    auto format = key(L"SOFTWARE\\Webcamoid\\VirtualCamera\\Cameras\\1\\Formats\\1");
    text(format, L"format", L"YUY2"); text(format, L"fps", L"30/1"); number(format, L"width", 1280); number(format, L"height", 720);
    assert(prepareDeviceFormat(1280, 720, false));
    assert(!prepareDeviceFormat(1920, 1080, false));
    assert(prepareDeviceFormat(1920, 1080, true));
    assert(prepareDeviceFormat(1920, 1080, false));
    text(format, L"format", L"RGB24"); assert(!prepareDeviceFormat(1280, 720, true));
    RegCloseKey(format);
    assert(RegOverridePredefKey(HKEY_LOCAL_MACHINE, nullptr) == ERROR_SUCCESS);
    RegCloseKey(fixture);
    assert(RegDeleteTreeW(HKEY_CURRENT_USER, testRoot.c_str()) == ERROR_SUCCESS);
}
