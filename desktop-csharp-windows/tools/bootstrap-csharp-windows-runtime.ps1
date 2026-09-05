[CmdletBinding(SupportsShouldProcess, ConfirmImpact = 'Medium')]
param(
    [int]$RequiredMajor = 10,

    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$RuntimeVersion = '10.0.0',

    [string]$InstallerPath,

    [switch]$Download,

    [string]$DownloadUri,

    [string]$Sha256,

    [switch]$Install
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [OperatingSystem]::IsWindows()) {
    throw 'This runtime bootstrapper only supports Windows x64.'
}

if ($RequiredMajor -lt 1 -or $RequiredMajor -gt 99) {
    throw 'RequiredMajor must be between 1 and 99.'
}

$maxOutputBytes = 256KB
$maxInstallerBytes = 350MB
$approvedDownloadHosts = @(
    'dotnetcli.azureedge.net',
    'download.visualstudio.microsoft.com',
    'builds.dotnet.microsoft.com',
    'dotnet.microsoft.com'
)

function Get-DotnetPath {
    $candidates = @()
    $programFiles = [Environment]::GetFolderPath('ProgramFiles')
    if (-not [string]::IsNullOrWhiteSpace($programFiles)) {
        $candidates += Join-Path $programFiles 'dotnet\dotnet.exe'
    }

    $command = Get-Command 'dotnet.exe' -ErrorAction SilentlyContinue
    if ($null -ne $command -and -not [string]::IsNullOrWhiteSpace($command.Source)) {
        $candidates += $command.Source
    }

    foreach ($candidate in $candidates | Select-Object -Unique) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            $item = Get-Item -LiteralPath $candidate
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) {
                return [IO.Path]::GetFullPath($candidate)
            }
        }
    }

    return $null
}

function Get-RuntimeProbe {
    $dotnet = Get-DotnetPath
    if ([string]::IsNullOrWhiteSpace($dotnet)) {
        return [pscustomobject]@{
            code = 'dotnet_missing'
            dotnet_path = $null
            versions = @()
            ready = $false
        }
    }

    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $dotnet
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $null = $start.ArgumentList.Add('--list-runtimes')
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) {
            throw 'dotnet process did not start.'
        }

        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(5000)) {
            try { $process.Kill($true) } catch { }
            throw 'dotnet --list-runtimes timed out.'
        }

        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($stdout.Length -gt $maxOutputBytes -or $stderr.Length -gt $maxOutputBytes) {
            throw 'dotnet runtime output exceeded the bounded limit.'
        }
        if ($process.ExitCode -ne 0) {
            throw "dotnet --list-runtimes failed with exit code $($process.ExitCode)."
        }

        $versions = @(
            foreach ($line in ($stdout -split "`r?`n")) {
                if ($line -match '^\s*Microsoft\.WindowsDesktop\.App\s+(\d+)\.(\d+)\.(\d+)\s+\[(.+)\]\s*$') {
                    [pscustomobject]@{
                        major = [int]$Matches[1]
                        minor = [int]$Matches[2]
                        patch = [int]$Matches[3]
                        path = $Matches[4]
                    }
                }
            }
        )
        $ready = @($versions | Where-Object { $_.major -eq $RequiredMajor }).Count -gt 0
        [pscustomobject]@{
            code = if ($ready) { 'ready' } else { 'runtime_missing' }
            dotnet_path = $dotnet
            versions = $versions
            ready = $ready
        }
    }
    catch {
        [pscustomobject]@{
            code = 'probe_failed'
            dotnet_path = $dotnet
            versions = @()
            ready = $false
            error = $_.Exception.Message
        }
    }
    finally {
        $process.Dispose()
    }
}

function Resolve-Installer([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw 'InstallerPath cannot be empty.'
    }

    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw 'InstallerPath must point to an existing file.'
    }
    $item = Get-Item -LiteralPath $resolved
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'InstallerPath cannot be a reparse point.'
    }
    if ($item.Length -le 0 -or $item.Length -gt $maxInstallerBytes) {
        throw 'Runtime installer size is outside the bounded limit.'
    }
    return $resolved
}

function Assert-Installer([string]$Path, [string]$ExpectedSha256) {
    $resolved = Resolve-Installer $Path
    $item = Get-Item -LiteralPath $resolved
    $hash = (Get-FileHash -LiteralPath $resolved -Algorithm SHA256).Hash.ToLowerInvariant()
    if (-not [string]::IsNullOrWhiteSpace($ExpectedSha256) -and $hash -ne $ExpectedSha256.Trim().ToLowerInvariant()) {
        throw 'Runtime installer SHA-256 does not match the expected value.'
    }

    $signature = Get-AuthenticodeSignature -LiteralPath $resolved
    if ($signature.Status -ne 'Valid') {
        throw "Runtime installer Authenticode status is not Valid: $($signature.Status)."
    }

    return [pscustomobject]@{
        path = $resolved
        size_bytes = $item.Length
        sha256 = $hash
        signature_status = [string]$signature.Status
    }
}

function Resolve-DownloadUri([string]$Candidate) {
    $value = if ([string]::IsNullOrWhiteSpace($Candidate)) {
        "https://dotnetcli.azureedge.net/dotnet/WindowsDesktop/$RuntimeVersion/windowsdesktop-runtime-$RuntimeVersion-win-x64.exe"
    }
    else {
        $Candidate
    }

    $uri = [Uri]::new($value)
    if ($uri.Scheme -ne 'https' -or ($uri.Port -ne -1 -and $uri.Port -ne 443) -or $approvedDownloadHosts -notcontains $uri.Host.ToLowerInvariant()) {
        throw 'DownloadUri must use HTTPS on an approved Microsoft .NET host.'
    }
    return $uri
}

function Download-Bounded([Uri]$Uri, [string]$ExpectedSha256) {
    if ([string]::IsNullOrWhiteSpace($ExpectedSha256) -or $ExpectedSha256 -notmatch '^[0-9a-fA-F]{64}$') {
        throw 'A 64-character SHA-256 value is required before an online download.'
    }

    $destination = Join-Path ([IO.Path]::GetTempPath()) ('windowsdesktop-runtime-' + [Guid]::NewGuid().ToString('N') + '.exe')
    $handler = [Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $client = [Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(45)
    $response = $null
    $input = $null
    $output = $null
    try {
        $response = $client.GetAsync($Uri, [Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
        if (-not $response.IsSuccessStatusCode) {
            throw "Runtime download returned HTTP $([int]$response.StatusCode)."
        }
        if ($response.Content.Headers.ContentLength -gt $maxInstallerBytes) {
            throw 'Runtime download exceeds the bounded installer size.'
        }

        $input = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
        $output = [IO.File]::Create($destination)
        $buffer = [byte[]]::new(64KB)
        [long]$total = 0
        while (($read = $input.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $total += $read
            if ($total -gt $maxInstallerBytes) {
                throw 'Runtime download exceeded the bounded installer size.'
            }
            $output.Write($buffer, 0, $read)
        }
        $output.Flush()
        $checked = Assert-Installer $destination $ExpectedSha256
        return $checked
    }
    catch {
        if (Test-Path -LiteralPath $destination) {
            Remove-Item -LiteralPath $destination -Force -ErrorAction SilentlyContinue
        }
        throw
    }
    finally {
        if ($null -ne $output) { $output.Dispose() }
        if ($null -ne $input) { $input.Dispose() }
        if ($null -ne $response) { $response.Dispose() }
        $client.Dispose()
        $handler.Dispose()
    }
}

$probe = Get-RuntimeProbe
if ($probe.ready) {
    [ordered]@{
        schema_version = 1
        required_major = $RequiredMajor
        status = 'ready'
        probe = $probe
    } | ConvertTo-Json -Depth 6
    return
}

if (-not $Install -and [string]::IsNullOrWhiteSpace($InstallerPath) -and -not $Download) {
    [ordered]@{
        schema_version = 1
        required_major = $RequiredMajor
        status = $probe.code
        probe = $probe
        action = 'install_required'
    } | ConvertTo-Json -Depth 6
    return
}

if ($Download -and -not [string]::IsNullOrWhiteSpace($InstallerPath)) {
    throw 'InstallerPath and Download cannot be used together.'
}
if ($Download -and -not $Install) {
    throw 'Download requires -Install so a temporary verified installer is not discarded.'
}

$installer = $null
$temporaryInstaller = $false
try {
    if ($Download) {
        if ($WhatIfPreference) {
            $uri = Resolve-DownloadUri $DownloadUri
            [ordered]@{
                schema_version = 1
                required_major = $RequiredMajor
                status = 'what_if'
                action = 'download_and_install'
                download_uri = $uri.AbsoluteUri
                install = $Install.IsPresent
            } | ConvertTo-Json -Depth 6
            return
        }
        $uri = Resolve-DownloadUri $DownloadUri
        $installer = (Download-Bounded $uri $Sha256).path
        $temporaryInstaller = $true
    }
    elseif (-not [string]::IsNullOrWhiteSpace($InstallerPath)) {
        $installer = (Assert-Installer $InstallerPath $Sha256).path
    }
    else {
        [ordered]@{
            schema_version = 1
            required_major = $RequiredMajor
            status = 'runtime_missing'
            probe = $probe
            action = 'install_required'
        } | ConvertTo-Json -Depth 6
        return
    }

    if (-not $Install) {
        [ordered]@{
            schema_version = 1
            required_major = $RequiredMajor
            status = 'installer_ready'
            installer = $installer
            probe = $probe
        } | ConvertTo-Json -Depth 6
        return
    }

    if (-not $PSCmdlet.ShouldProcess($installer, 'Install .NET Windows Desktop Runtime')) {
        [ordered]@{
            schema_version = 1
            required_major = $RequiredMajor
            status = 'cancelled'
            installer = $installer
        } | ConvertTo-Json -Depth 6
        return
    }

    $arguments = @('/install', '/quiet', '/norestart')
    $startParameters = @{
        FilePath = $installer
        ArgumentList = $arguments
        Wait = $true
        PassThru = $true
    }
    $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        $startParameters.Verb = 'RunAs'
    }
    $process = Start-Process @startParameters
    if ($process.ExitCode -notin @(0, 3010)) {
        throw "Runtime installer failed with exit code $($process.ExitCode)."
    }

    $after = Get-RuntimeProbe
    if (-not $after.ready) {
        throw 'Runtime installer completed but the required Windows Desktop Runtime was not detected.'
    }
    [ordered]@{
        schema_version = 1
        required_major = $RequiredMajor
        status = if ($process.ExitCode -eq 3010) { 'installed_reboot_required' } else { 'installed' }
        installer_exit_code = $process.ExitCode
        probe = $after
    } | ConvertTo-Json -Depth 6
}
finally {
    if ($temporaryInstaller -and $null -ne $installer -and (Test-Path -LiteralPath $installer)) {
        Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
    }
}
