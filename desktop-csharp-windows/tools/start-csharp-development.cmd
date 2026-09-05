@echo off
setlocal

set "AUTOLIVE_CONTROL_PLANE_ENV=development"
set "AUTOLIVE_CONTROL_PLANE_BASE_URI=http://101.96.208.132:9090"
set "APP_EXE=%~dp0..\src\GpAutoLive.App\bin\x64\Release\net10.0-windows10.0.19041.0\win-x64\GpAutoLive.exe"
set "DOTNET_EXE=%~dp0..\.tools\dotnet\dotnet.exe"

if not "%~1"=="" goto :runtime_argument
set "MEDIA_RUNTIME_ROOT=%~dp0..\artifacts\csharp-gpu83-real-v2"
if exist "%MEDIA_RUNTIME_ROOT%\runtime\media\1.0.0\manifest.json" goto :runtime_ready
set "MEDIA_RUNTIME_ROOT=%~dp0..\artifacts\csharp-windows-controller-20260903-v90"
goto :runtime_ready

:runtime_argument
for %%I in ("%~1") do set "MEDIA_RUNTIME_ROOT=%%~fI"

:runtime_ready

if not exist "%MEDIA_RUNTIME_ROOT%\runtime\media\1.0.0\manifest.json" (
    echo 未找到已校验的外置媒体运行时：%MEDIA_RUNTIME_ROOT%
    echo 请将发布包根目录作为第一个参数传入，或先生成 csharp-gpu83-real-v2 / csharp-windows-controller-20260903-v90。
    exit /b 1
)

set "AUTOLIVE_MEDIA_RUNTIME_ROOT=%MEDIA_RUNTIME_ROOT%"

if not exist "%DOTNET_EXE%" (
    echo 未找到项目锁定的本地 .NET SDK：%DOTNET_EXE%
    echo 请先准备 desktop-csharp-windows\.tools\dotnet，禁止使用服务器构建。
    exit /b 1
)

"%DOTNET_EXE%" build "%~dp0..\src\GpAutoLive.App\GpAutoLive.App.csproj" -c Release -p:Platform=x64 --no-restore --verbosity minimal
if errorlevel 1 (
    echo CSharp app local Release x64 build failed.
    exit /b 1
)

if not exist "%APP_EXE%" (
    echo CSharp app build output was not found after local build: %APP_EXE%
    exit /b 1
)

start "" "%APP_EXE%"
endlocal
