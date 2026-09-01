/*
 * AkVirtualCamera sidecar for GpAutoLive.
 *
 * This file is distributed under GPL-3.0-or-later as an independent
 * component. It does not link the Rust desktop process to AkVirtualCamera.
 * The sidecar loads the upstream C API from the fixed application directory,
 * accepts only the fixed YUY2 1280x720 frame protocol, and serves one
 * current-user named pipe per authenticated session token. The token is
 * received once over inherited stdin so it never appears in argv/env.
 */

#include <windows.h>
#include <sddl.h>

#include <array>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <cwchar>
#include <limits>
#include <string>
#include <vector>

#ifndef PIPE_REJECT_REMOTE_CLIENTS
#define PIPE_REJECT_REMOTE_CLIENTS 0x00000008
#endif

namespace {

constexpr std::size_t kTokenHexLength = 32;
constexpr std::size_t kTokenBytes = 16;
constexpr std::size_t kFrameHeaderBytes = 52;
constexpr std::size_t kFramePayloadBytes = 1280u * 720u * 2u;
constexpr std::uint16_t kProtocolVersion = 1;
constexpr std::uint32_t kWidth = 1280;
constexpr std::uint32_t kHeight = 720;
constexpr char kFrameMagic[] = "GPAKVC01";
constexpr char kDeviceId[] = "GpAutoLiveCamera";
constexpr char kDeviceDescription[] = "GpAutoLive Camera";
constexpr wchar_t kPipePrefix[] = L"\\\\.\\pipe\\GpAutoLive-AkVirtualCamera-";

using VcamHandle = void *;
using VcamOpen = VcamHandle (*)();
using VcamClose = void (*)(VcamHandle);
using VcamDevices = int (*)(VcamHandle, char *, std::size_t *);
using VcamDescription = int (*)(VcamHandle, const char *, char *, std::size_t *);
using VcamDataMode = int (*)(VcamHandle, char *, std::size_t);
using VcamDirectMode = int (*)(VcamHandle, const char *, bool *);
using VcamStreamStart = int (*)(VcamHandle, const char *);
using VcamStreamSend = int (*)(VcamHandle,
                               const char *,
                               const char *,
                               int,
                               int,
                               const char **,
                               std::size_t *);
using VcamStreamStop = int (*)(VcamHandle, const char *);
using VcamClients = int (*)(VcamHandle, std::uint64_t *, std::size_t);

struct VcamApi {
    HMODULE module = nullptr;
    VcamOpen open = nullptr;
    VcamClose close = nullptr;
    VcamDevices devices = nullptr;
    VcamDescription description = nullptr;
    VcamDataMode dataMode = nullptr;
    VcamDirectMode directMode = nullptr;
    VcamStreamStart streamStart = nullptr;
    VcamStreamSend streamSend = nullptr;
    VcamStreamStop streamStop = nullptr;
    VcamClients clients = nullptr;

    bool load() {
        module = LoadLibraryExW(L"vcam_capi.dll", nullptr, LOAD_LIBRARY_SEARCH_APPLICATION_DIR);
        if (!module)
            return false;
        open = loadFunction<VcamOpen>("vcam_open");
        close = loadFunction<VcamClose>("vcam_close");
        devices = loadFunction<VcamDevices>("vcam_devices");
        description = loadFunction<VcamDescription>("vcam_description");
        dataMode = loadFunction<VcamDataMode>("vcam_data_mode");
        directMode = loadFunction<VcamDirectMode>("vcam_direct_mode");
        streamStart = loadFunction<VcamStreamStart>("vcam_stream_start");
        streamSend = loadFunction<VcamStreamSend>("vcam_stream_send");
        streamStop = loadFunction<VcamStreamStop>("vcam_stream_stop");
        clients = loadFunction<VcamClients>("vcam_clients");
        if (!open || !close || !devices || !description || !dataMode || !directMode
            || !streamStart || !streamSend || !streamStop || !clients) {
            FreeLibrary(module);
            module = nullptr;
            return false;
        }
        return true;
    }

    ~VcamApi() {
        if (module)
            FreeLibrary(module);
    }

private:
    template <typename Function>
    Function loadFunction(const char *name) const {
        return reinterpret_cast<Function>(GetProcAddress(module, name));
    }
};

struct CameraSession {
    VcamApi api;
    VcamHandle handle = nullptr;
    std::string deviceId;
    bool streaming = false;

    bool start() {
        if (!api.load())
            return false;
        handle = api.open();
        if (!handle)
            return false;
        if (!findDevice())
            return false;
        // These settings are privileged AkVCam configuration and are applied
        // once by the elevated installer.  The sidecar stays unelevated and
        // only verifies them here; calling the setters on every start would
        // trigger a UAC prompt and break unattended output recovery.
        char mode[16] = {};
        if (api.dataMode(handle, mode, sizeof(mode)) != 0 || std::strcmp(mode, "mmap") != 0)
            return false;
        bool directMode = false;
        if (api.directMode(handle, deviceId.c_str(), &directMode) != 0 || !directMode)
            return false;
        if (api.streamStart(handle, deviceId.c_str()) != 0)
            return false;
        streaming = true;
        return true;
    }

    ~CameraSession() {
        if (streaming)
            api.streamStop(handle, deviceId.c_str());
        if (handle)
            api.close(handle);
    }

    bool send(const std::vector<std::uint8_t> &payload) {
        if (!streaming || payload.size() != kFramePayloadBytes)
            return false;
        const char *planes[] = {reinterpret_cast<const char *>(payload.data())};
        std::size_t lineSizes[] = {kWidth * 2u};
        return api.streamSend(handle,
                              deviceId.c_str(),
                              "YUY2",
                              static_cast<int>(kWidth),
                              static_cast<int>(kHeight),
                              planes,
                              lineSizes)
            == 0;
    }

    int clientCount() const {
        if (!handle || !api.clients)
            return -1;
        return api.clients(handle, nullptr, 0);
    }

private:
    bool findDevice() {
        std::size_t required = 0;
        const int count = api.devices(handle, nullptr, &required);
        if (count < 1 || required == 0 || required > 64u * 1024u)
            return false;
        std::vector<char> ids(required, '\0');
        if (api.devices(handle, ids.data(), &required) < 1)
            return false;
        for (std::size_t offset = 0; offset < ids.size() && ids[offset] != '\0';) {
            const char *id = ids.data() + offset;
            const std::size_t remaining = ids.size() - offset;
            std::size_t idLength = 0;
            while (idLength < remaining && id[idLength] != '\0')
                ++idLength;
            if (idLength == remaining)
                return false;
            // 只接受本产品安装器创建的固定设备 ID；不能因为其他软件使用了
            // 相同描述就把画面写入第三方 AkVirtualCamera 实例。
            if (std::string(id, idLength) != kDeviceId) {
                offset += idLength + 1;
                continue;
            }
            std::size_t descriptionSize = 0;
            if (api.description(handle, id, nullptr, &descriptionSize) == 0
                && descriptionSize > 0 && descriptionSize <= 4096) {
                std::vector<char> description(descriptionSize, '\0');
                if (api.description(handle, id, description.data(), &descriptionSize) >= 0
                    && std::string(description.data()) == kDeviceDescription) {
                    deviceId.assign(id, idLength);
                    return true;
                }
            }
            offset += idLength + 1;
        }
        return false;
    }
};

void emitClientCount(const CameraSession &session) {
    const int count = session.clientCount();
    // stdout is a private pipe owned by the Rust parent; do not expose paths,
    // PIDs or any other client data, only the bounded count needed for status.
    if (count >= 0 && count <= 1024) {
        std::printf("GPAKVC_CLIENTS %d\n", count);
        std::fflush(stdout);
    }
}

bool readExact(HANDLE pipe, void *buffer, std::size_t size) {
    auto *cursor = static_cast<std::uint8_t *>(buffer);
    while (size > 0) {
        const DWORD request = static_cast<DWORD>(size > std::numeric_limits<DWORD>::max()
                                                     ? std::numeric_limits<DWORD>::max()
                                                     : size);
        DWORD received = 0;
        if (!ReadFile(pipe, cursor, request, &received, nullptr) || received == 0)
            return false;
        cursor += received;
        size -= received;
    }
    return true;
}

std::uint16_t readU16(const std::uint8_t *data) {
    return static_cast<std::uint16_t>(data[0])
        | static_cast<std::uint16_t>(data[1]) << 8;
}

std::uint32_t readU32(const std::uint8_t *data) {
    return static_cast<std::uint32_t>(data[0])
        | static_cast<std::uint32_t>(data[1]) << 8
        | static_cast<std::uint32_t>(data[2]) << 16
        | static_cast<std::uint32_t>(data[3]) << 24;
}

std::uint64_t readU64(const std::uint8_t *data) {
    std::uint64_t value = 0;
    for (unsigned int index = 0; index < 8; ++index)
        value |= static_cast<std::uint64_t>(data[index]) << (index * 8);
    return value;
}

std::int64_t readI64(const std::uint8_t *data) {
    return static_cast<std::int64_t>(readU64(data));
}

int hexValue(wchar_t value) {
    if (value >= L'0' && value <= L'9')
        return value - L'0';
    if (value >= L'a' && value <= L'f')
        return value - L'a' + 10;
    if (value >= L'A' && value <= L'F')
        return value - L'A' + 10;
    return -1;
}

bool parseSessionToken(const wchar_t *value, std::array<std::uint8_t, kTokenBytes> &token) {
    if (!value || wcslen(value) != kTokenHexLength)
        return false;
    bool nonZero = false;
    for (std::size_t index = 0; index < kTokenBytes; ++index) {
        const int high = hexValue(value[index * 2]);
        const int low = hexValue(value[index * 2 + 1]);
        if (high < 0 || low < 0)
            return false;
        token[index] = static_cast<std::uint8_t>((high << 4) | low);
        nonZero = nonZero || token[index] != 0;
    }
    return nonZero;
}

bool readSessionTokenFromStdin(std::array<std::uint8_t, kTokenBytes> &token) {
    std::array<wchar_t, kTokenHexLength + 1> value{};
    std::size_t index = 0;
    const int first = std::fgetc(stdin);
    if (first == EOF)
        return false;
    // Windows PowerShell 5's redirected StreamWriter emits a UTF-8 BOM before
    // the first write. Accept that framing marker, but never skip arbitrary
    // bytes: the token remains exactly 32 ASCII hex characters.
    if (first == 0xEF) {
        if (std::fgetc(stdin) != 0xBB || std::fgetc(stdin) != 0xBF)
            return false;
    } else {
        value[index++] = static_cast<wchar_t>(first);
    }
    for (; index < kTokenHexLength; ++index) {
        const int character = std::fgetc(stdin);
        if (character == EOF)
            return false;
        value[index] = static_cast<wchar_t>(character);
    }
    const int terminator = std::fgetc(stdin);
    if (terminator == '\r') {
        const int lineFeed = std::fgetc(stdin);
        if (lineFeed != '\n' && lineFeed != EOF)
            return false;
    } else if (terminator != '\n' && terminator != EOF) {
        return false;
    }
    return parseSessionToken(value.data(), token);
}

std::wstring pipeName(const std::array<std::uint8_t, kTokenBytes> &token) {
    static constexpr wchar_t digits[] = L"0123456789abcdef";
    std::wstring name(kPipePrefix);
    name.reserve(name.size() + kTokenHexLength);
    for (const auto byte : token) {
        name.push_back(digits[(byte >> 4) & 0x0f]);
        name.push_back(digits[byte & 0x0f]);
    }
    return name;
}

HANDLE createPipe(const std::wstring &name, PSECURITY_DESCRIPTOR *descriptor) {
    *descriptor = nullptr;
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
            L"D:P(A;;GA;;;OW)",
            SDDL_REVISION_1,
            descriptor,
            nullptr))
        return INVALID_HANDLE_VALUE;
    SECURITY_ATTRIBUTES attributes{};
    attributes.nLength = sizeof(attributes);
    attributes.lpSecurityDescriptor = *descriptor;
    HANDLE pipe = CreateNamedPipeW(name.c_str(),
                                   PIPE_ACCESS_INBOUND,
                                   PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT
                                       | PIPE_REJECT_REMOTE_CLIENTS,
                                   1,
                                   0,
                                   static_cast<DWORD>(kFrameHeaderBytes + kFramePayloadBytes),
                                   5000,
                                   &attributes);
    if (pipe == INVALID_HANDLE_VALUE) {
        LocalFree(*descriptor);
        *descriptor = nullptr;
    }
    return pipe;
}

bool consumeFrames(HANDLE pipe, CameraSession &session) {
    std::array<std::uint8_t, kFrameHeaderBytes> header{};
    std::vector<std::uint8_t> payload(kFramePayloadBytes);
    std::uint64_t currentGeneration = 0;
    std::uint64_t lastSequence = 0;
    auto lastStatusAt = std::chrono::steady_clock::now();
    while (readExact(pipe, header.data(), header.size())) {
        if (std::memcmp(header.data(), kFrameMagic, 8) != 0
            || readU16(header.data() + 8) != kProtocolVersion
            || readU16(header.data() + 10) != kFrameHeaderBytes
            || readU32(header.data() + 36) != kWidth
            || readU32(header.data() + 40) != kHeight
            || readU32(header.data() + 44) != kFramePayloadBytes
            || readU32(header.data() + 48) != 0
            || readU64(header.data() + 12) == 0
            || readU64(header.data() + 20) == 0
            || readI64(header.data() + 28) < 0)
            return false;

        const std::uint64_t generation = readU64(header.data() + 12);
        const std::uint64_t sequence = readU64(header.data() + 20);
        if (!readExact(pipe, payload.data(), payload.size()))
            return false;
        if (generation < currentGeneration || (generation == currentGeneration && sequence <= lastSequence))
            continue;
        if (generation != currentGeneration)
            lastSequence = 0;
        if (!session.send(payload))
            return false;
        currentGeneration = generation;
        lastSequence = sequence;
        const auto now = std::chrono::steady_clock::now();
        if (now - lastStatusAt >= std::chrono::milliseconds(250)) {
            emitClientCount(session);
            lastStatusAt = now;
        }
    }
    return true;
}

} // namespace

int wmain(int argc, wchar_t **argv) {
    if (argc != 2 || wcscmp(argv[1], L"--session-token-stdin") != 0)
        return 2;
    std::array<std::uint8_t, kTokenBytes> token{};
    if (!readSessionTokenFromStdin(token))
        return 2;

    CameraSession cameraSession;
    PSECURITY_DESCRIPTOR descriptor = nullptr;
    HANDLE pipe = createPipe(pipeName(token), &descriptor);
    if (pipe == INVALID_HANDLE_VALUE) {
        return 3;
    }
    // Initialize the fixed device before accepting the producer connection so that
    // a connected pipe means the DirectShow endpoint can receive YUY2 frames.
    if (!cameraSession.start()) {
        CloseHandle(pipe);
        LocalFree(descriptor);
        return 5;
    }
    const BOOL connected = ConnectNamedPipe(pipe, nullptr)
        ? TRUE
        : (GetLastError() == ERROR_PIPE_CONNECTED ? TRUE : FALSE);
    if (!connected) {
        CloseHandle(pipe);
        LocalFree(descriptor);
        return 4;
    }

    emitClientCount(cameraSession);
    const bool consumed = consumeFrames(pipe, cameraSession);
    FlushFileBuffers(pipe);
    DisconnectNamedPipe(pipe);
    CloseHandle(pipe);
    LocalFree(descriptor);
    return consumed ? 0 : 5;
}
