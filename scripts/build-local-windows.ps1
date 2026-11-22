param(
    [ValidateSet('Fast', 'Full')]
    [string]$Mode = 'Full'
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# This shared builder lives in scripts/; both clickable launchers live in the repository root.
$root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $root
$log = Join-Path $root 'local-build.log'
Start-Transcript -Path $log -Force | Out-Null
$overridePath = Join-Path $root 'desktop\src-tauri\tauri.local-build.conf.json'
$exitCode = 0
try {
    function Run($exe, [string[]]$arguments) {
        Write-Host "`n> $exe $($arguments -join ' ')" -ForegroundColor Cyan
        & $exe @arguments
        if ($LASTEXITCODE -ne 0) { throw "Command failed (exit $LASTEXITCODE): $exe $($arguments -join ' ')" }
    }
    Write-Host "YuE2 Studio - $($Mode.ToUpperInvariant()) Windows Portable Builder" -ForegroundColor Green
    Write-Host "Mode: $Mode" -ForegroundColor Yellow
    Write-Host "Repository: $root"
    foreach ($file in @('app\package-lock.json','desktop\package-lock.json','desktop\src-tauri\tauri.conf.json')) {
        if (-not (Test-Path -LiteralPath (Join-Path $root $file))) { throw "Repository file missing: $file. Put FAST-BUILD.bat and FULL-BUILD.bat in the repository root." }
    }
    foreach ($tool in @('node.exe','npm.cmd','cargo.exe','rustc.exe','git.exe')) {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "Missing command: $tool" }
    }
    # The launcher initializes VS's x64 developer tools first. Refuse to waste
    # time re-installing npm packages if the Windows linker is still missing.
    $linker = Get-Command 'link.exe' -ErrorAction SilentlyContinue
    if (-not $linker) {
        throw 'MSVC link.exe is not on PATH. Start FAST-BUILD.bat or FULL-BUILD.bat (not the PowerShell script directly). If it still fails, repair Desktop development with C++ in Visual Studio Installer.'
    }
    Write-Host "MSVC linker: $($linker.Source)" -ForegroundColor Green
    $branch = (& git branch --show-current).Trim()
    Write-Host "Current Git branch: $branch" -ForegroundColor Yellow
    # All DJ changes now live on the fork's main branch: validate the files, not the branch name.
    if (-not (Test-Path -LiteralPath (Join-Path $root 'app\components\player\DjMode.tsx'))) {
        throw 'DJ mode source is missing. Fetch and pull your fork main in GitHub Desktop.'
    }
    # Remember the exact package manifests that were installed in node_modules.
    # In Fast mode npm ci runs only when dependencies are missing or changed.
    # Full mode always refreshes dependencies to verify the lockfiles.
    function Ensure-Dependencies([string]$package) {
        $folder = Join-Path $root $package
        $modules = Join-Path $folder 'node_modules'
        $stamp = Join-Path $modules '.yue2-local-build-lock'
        $fingerprint = (Get-FileHash -LiteralPath (Join-Path $folder 'package.json') -Algorithm SHA256).Hash + ':' +
            (Get-FileHash -LiteralPath (Join-Path $folder 'package-lock.json') -Algorithm SHA256).Hash
        if ($Mode -eq 'Fast' -and (Test-Path -LiteralPath $stamp)) {
            $previous = (Get-Content -LiteralPath $stamp -Raw).Trim()
            if ($previous -eq $fingerprint) {
                Write-Host "[YuE2] Using cached $package dependencies (lockfiles unchanged)." -ForegroundColor Green
                return
            }
        }
        Run 'npm.cmd' @('--prefix', $package, 'ci', '--no-audit', '--no-fund')
        if (-not (Test-Path -LiteralPath $modules)) { throw "npm ci did not create $modules" }
        Set-Content -LiteralPath $stamp -Value $fingerprint -Encoding ASCII
    }
    Ensure-Dependencies 'app'
    Ensure-Dependencies 'desktop'
    $resourceRoot = Join-Path $root 'desktop\src-tauri\resources'
    $serverExe = Join-Path $resourceRoot 'yue2-cpp\yue-server.exe'
    $vstRoot = Join-Path $resourceRoot 'vst-host'
    if (-not ((Test-Path -LiteralPath $serverExe) -and (Test-Path -LiteralPath $vstRoot))) {
        Write-Host "`nDownloading upstream native runtime (not compiling CUDA/Vulkan engine)..." -ForegroundColor Yellow
        $headers = @{ 'User-Agent'='YuE2-local-builder'; 'Accept'='application/vnd.github+json' }
        $release = Invoke-RestMethod -Uri 'https://api.github.com/repos/timoncool/YuE2-Studio/releases/latest' -Headers $headers
        $asset = $release.assets | Where-Object { $_.name -match 'portable.*windows.*x64\.zip$' } | Select-Object -First 1
        if (-not $asset) { throw 'Latest upstream release has no portable Windows x64 ZIP. Native engine must be staged manually.' }
        $tempDir = Join-Path $env:TEMP ('yue2-native-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
        try {
            $zip = Join-Path $tempDir 'upstream.zip'
            $extract = Join-Path $tempDir 'extracted'
            Write-Host "Downloading $($asset.name) from upstream release $($release.tag_name)..."
            Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip
            Expand-Archive -LiteralPath $zip -DestinationPath $extract -Force
            foreach ($name in @('yue2-cpp','vst-host')) {
                $source = Get-ChildItem -LiteralPath $extract -Directory -Recurse | Where-Object { $_.Name -eq $name -and $_.Parent.Name -eq 'resources' } | Select-Object -First 1
                if (-not $source) { throw "Upstream portable archive is missing resources/$name" }
                $dest = Join-Path $resourceRoot $name
                New-Item -ItemType Directory -Path $resourceRoot -Force | Out-Null
                if (Test-Path -LiteralPath $dest) { Remove-Item -LiteralPath $dest -Recurse -Force }
                Copy-Item -LiteralPath $source.FullName -Destination $dest -Recurse -Force
            }
        } finally {
            if (Test-Path -LiteralPath $tempDir) { Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue }
        }
    } else {
        Write-Host 'Existing native runtime found; skipping upstream download.' -ForegroundColor Green
    }
    if (-not (Test-Path -LiteralPath $serverExe)) { throw "Native server missing: $serverExe" }
    if (-not (Test-Path -LiteralPath $vstRoot)) { throw "VST host directory missing: $vstRoot" }
    if ($Mode -eq 'Full') {
        Write-Host "[YuE2] Full build: running all frontend tests." -ForegroundColor Yellow
        Run 'npm.cmd' @('--prefix','app','run','test')
    } else {
        Write-Host '[YuE2] Fast build: skipping the full frontend test suite.' -ForegroundColor Yellow
    }
    # Tauri's beforeBuildCommand runs npm --prefix ../app run build.
    # Do NOT build here too: that used to compile the same frontend twice.
    $override = @{
        bundle = @{
            createUpdaterArtifacts = $false
            resources = @{
                'resources/yue2-cpp' = 'resources/yue2-cpp'
                'resources/vst-host' = 'resources/vst-host'
            }
        }
        plugins = @{ updater = @{ endpoints = @() } }
    }
    $override | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $overridePath -Encoding utf8
    Push-Location (Join-Path $root 'desktop')
    try { Run 'npx.cmd' @('tauri','build','--no-bundle','--config',$overridePath) }
    finally { Pop-Location }
    $config = Get-Content -LiteralPath (Join-Path $root 'desktop\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
    $version = [string]$config.version
    $binary = [string]$config.mainBinaryName
    if (-not $binary) { throw 'mainBinaryName is missing from desktop/src-tauri/tauri.conf.json' }
    $exe = Join-Path $root "desktop\src-tauri\target\release\$binary.exe"
    if (-not (Test-Path -LiteralPath $exe)) { throw "Compiled desktop EXE not found: $exe" }
    $dist = Join-Path $root 'LOCAL-BUILDS'
    $portableName = "YuE2-Studio-$version-portable-windows-x64"
    $stage = Join-Path $dist $portableName
    $archive = Join-Path $dist "$portableName.zip"
    New-Item -ItemType Directory -Path $dist -Force | Out-Null
    if ($Mode -eq 'Full' -and (Test-Path -LiteralPath $stage)) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
    New-Item -ItemType Directory -Path (Join-Path $stage 'resources') -Force | Out-Null
    Copy-Item -LiteralPath $exe -Destination $stage -Force
    foreach ($name in @('yue2-cpp', 'vst-host')) {
        $source = Join-Path $resourceRoot $name
        $destination = Join-Path (Join-Path $stage 'resources') $name
        if ($Mode -eq 'Fast' -and (Test-Path -LiteralPath $destination)) {
            # /MIR copies only changed files, but still removes stale runtime files.
            # Robocopy codes 0-7 indicate success; 8 or higher indicate failure.
            & robocopy.exe $source $destination /MIR /R:2 /W:1 /NFL /NDL /NJH /NJS /NP
            if ($LASTEXITCODE -ge 8) { throw "Could not refresh portable resource $name (robocopy exit $LASTEXITCODE)" }
        } else {
            Copy-Item -LiteralPath $source -Destination $destination -Recurse -Force
        }
    }
    New-Item -ItemType File -Path (Join-Path $stage 'portable.flag') -Force | Out-Null
    if ($Mode -eq 'Full') {
        if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
        Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive -CompressionLevel Optimal
    } elseif (Test-Path -LiteralPath $archive) {
        # Never leave an old ZIP that looks like it contains the new fast build.
        Remove-Item -LiteralPath $archive -Force
    }
    Write-Host "`n$($Mode.ToUpperInvariant()) BUILD COMPLETE!" -ForegroundColor Green
    Write-Host "Portable folder: $stage" -ForegroundColor Green
    if ($Mode -eq 'Full') {
        Write-Host "Portable ZIP:    $archive" -ForegroundColor Green
    } else {
        Write-Host 'Fast mode: ZIP skipped. Use FULL-BUILD.bat to produce a release archive.' -ForegroundColor Yellow
    }
    Write-Host 'Run the EXE inside the portable folder to test it.'
    Start-Process explorer.exe -ArgumentList "`"$dist`""
} catch {
    $exitCode = 1
    Write-Host "`nBUILD FAILED: $($_.Exception.Message)" -ForegroundColor Red
    Write-Host "Log: $log" -ForegroundColor Yellow
} finally {
    Remove-Item -LiteralPath $overridePath -Force -ErrorAction SilentlyContinue
    Stop-Transcript | Out-Null
    Read-Host 'Press Enter to close this window'
}
exit $exitCode
