#pragma once
#include <windows.h>
#include <cstdint>
#include <string>

// The test installer grants SetValue only on this product's Formats\\1 key.
// No upstream setter is called: those setters launch an elevated Manager.
inline bool prepareDeviceFormat(std::uint32_t width, std::uint32_t height, bool development) {
    constexpr auto root = L"SOFTWARE\\Webcamoid\\VirtualCamera";
    auto stringValue = [](HKEY key, const wchar_t *name, const wchar_t *expected, bool ignoreCase = false) {
        wchar_t value[1024]{}; DWORD bytes = sizeof(value);
        return RegGetValueW(key, nullptr, name, RRF_RT_REG_SZ, nullptr, value, &bytes) == ERROR_SUCCESS
            && (ignoreCase ? _wcsicmp(value, expected) : wcscmp(value, expected)) == 0;
    };
    auto number = [](HKEY key, const wchar_t *name, DWORD &value) {
        DWORD bytes = sizeof(value);
        return RegGetValueW(key, nullptr, name, RRF_RT_REG_DWORD, nullptr, &value, &bytes) == ERROR_SUCCESS;
    };
    HKEY formatKey{}; DWORD oldWidth{}, oldHeight{};
    wchar_t modulePath[32768]{};
    const DWORD moduleLength = GetModuleFileNameW(nullptr, modulePath, 32768);
    if (moduleLength == 0 || moduleLength >= 32768) return false;
    std::wstring packageRoot(modulePath);
    for (int level = 0; level < 2; ++level) {
        const auto slash = packageRoot.find_last_of(L"\\/");
        if (slash == std::wstring::npos) return false;
        packageRoot.resize(slash);
    }
    bool ok = true;
    for (const auto view : {KEY_WOW64_64KEY, KEY_WOW64_32KEY}) {
        HKEY owner = nullptr;
        if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, root, 0, KEY_READ | view, &owner) != ERROR_SUCCESS) return false;
        const bool owned = stringValue(owner, L"installPath", packageRoot.c_str(), true);
        RegCloseKey(owner);
        if (!owned) return false;
    }
    do {
        // Upstream x86 and x64 both explicitly use this preferences view.
        const REGSAM flag = KEY_WOW64_64KEY;
        HKEY cameras = nullptr;
        const auto camerasPath = std::wstring(root) + L"\\Cameras";
        if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, camerasPath.c_str(), 0, KEY_READ | flag, &cameras) != ERROR_SUCCESS) { ok = false; break; }
        DWORD count = 0; bool found = false;
        ok = number(cameras, L"size", count) && count > 0 && count <= 100;
        for (DWORD index = 1; index <= count && ok && !found; ++index) {
            HKEY camera = nullptr;
            auto cameraPath = camerasPath + L"\\" + std::to_wstring(index);
            if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, cameraPath.c_str(), 0, KEY_READ | flag, &camera) != ERROR_SUCCESS) { ok = false; break; }
            found = stringValue(camera, L"id", L"GpAutoLiveCamera");
            if (found) {
                ok = stringValue(camera, L"description", L"GpAutoLive Camera");
                HKEY formatsParent = nullptr; DWORD countFormats = 0;
                ok = ok && RegOpenKeyExW(camera, L"Formats", 0, KEY_READ | flag, &formatsParent) == ERROR_SUCCESS;
                ok = ok && number(formatsParent, L"size", countFormats) && countFormats == 1;
                if (formatsParent) RegCloseKey(formatsParent);
                auto formatPath = cameraPath + L"\\Formats\\1";
                ok = ok && RegOpenKeyExW(HKEY_LOCAL_MACHINE, formatPath.c_str(), 0,
                    KEY_QUERY_VALUE | (development ? KEY_SET_VALUE : 0) | flag, &formatKey) == ERROR_SUCCESS;
                ok = ok && stringValue(formatKey, L"format", L"YUY2")
                    && (stringValue(formatKey, L"fps", L"30") || stringValue(formatKey, L"fps", L"30/1"))
                    && number(formatKey, L"width", oldWidth) && number(formatKey, L"height", oldHeight);
            }
            RegCloseKey(camera);
        }
        RegCloseKey(cameras);
        ok = ok && found;
    } while (false);
    if (ok && !development)
        ok = oldWidth == width && oldHeight == height;
    if (ok && development) {
        ok = RegSetValueExW(formatKey, L"width", 0, REG_DWORD, reinterpret_cast<const BYTE *>(&width), sizeof(width)) == ERROR_SUCCESS
            && RegSetValueExW(formatKey, L"height", 0, REG_DWORD, reinterpret_cast<const BYTE *>(&height), sizeof(height)) == ERROR_SUCCESS;
        if (!ok) {
            bool restored = true;
            restored = (RegSetValueExW(formatKey, L"width", 0, REG_DWORD, reinterpret_cast<const BYTE *>(&oldWidth), sizeof(DWORD)) == ERROR_SUCCESS) && restored;
            restored = (RegSetValueExW(formatKey, L"height", 0, REG_DWORD, reinterpret_cast<const BYTE *>(&oldHeight), sizeof(DWORD)) == ERROR_SUCCESS) && restored;
            if (!restored) { std::fprintf(stderr, "GPAKVC_FORMAT_ROLLBACK_FAILED\n"); }
        }
    }
    if (formatKey) RegCloseKey(formatKey);
    return ok;
}


