# 文件职责：在隔离配置下验证真实窗口、托盘与重复启动的生命周期。
param([string]$Executable = "$PSScriptRoot/../target/debug/packporter.exe")
$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path -LiteralPath $Executable).Path
if (Get-Process packporter -ErrorAction SilentlyContinue) { throw '请先关闭已有 PackPorter，避免实例锁影响测试。' }
. "$PSScriptRoot/window_probe.ps1"
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ("packporter-tray-" + [guid]::NewGuid())
$oldConfig = $env:PACKPORTER_CONFIG_DIR
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
function Start-App([bool]$Followed) {
    $info = [Diagnostics.ProcessStartInfo]::new($exePath)
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardError = $true
    if ($Followed) { $info.ArgumentList.Add('--launcher-follow') }
    $process = [Diagnostics.Process]::Start($info)
    $owned.Add($process)
    return $process
}
function Wait-Visible($Process, [bool]$Visible) {
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $Process.Refresh()
        if ($Process.HasExited) { throw ("应用意外退出 ($($Process.ExitCode)): " + $Process.StandardError.ReadToEnd()) }
        if ((([TrayRuntimeNative]::MainWindow($Process.Id)) -ne 0) -eq $Visible) { return }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    $title = [Text.StringBuilder]::new(512)
    $class = [Text.StringBuilder]::new(512)
    [void][TrayRuntimeNative]::GetWindowText(([TrayRuntimeNative]::MainWindow($Process.Id)), $title, 512)
    [void][TrayRuntimeNative]::GetClassName(([TrayRuntimeNative]::MainWindow($Process.Id)), $class, 512)
    throw "窗口可见性未变为 $Visible：$title ($class)"
}
function Close-Window($Process) {
    $Process.Refresh()
    if (![TrayRuntimeNative]::PostMessage(([TrayRuntimeNative]::MainWindow($Process.Id)), 0x0010, [UIntPtr]::Zero, [IntPtr]::Zero)) { throw '关闭消息投递失败' }
}
try {
    New-Item -ItemType Directory -Path $testRoot | Out-Null
    $env:PACKPORTER_CONFIG_DIR = $testRoot
    '{"close_to_tray":true,"follow_launchers":false}' | Set-Content (Join-Path $testRoot 'config.json') -Encoding utf8
    $app = Start-App $true
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([TrayRuntimeNative]::TrayWindow() -eq [IntPtr]::Zero -and !$app.HasExited -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    Wait-Visible $app $false
    if ([TrayRuntimeNative]::TrayWindow() -eq [IntPtr]::Zero) { throw '静默启动缺少托盘入口' }
    $second = Start-App $false
    if (!$second.WaitForExit(5000)) { throw '重复启动未退出' }
    Wait-Visible $app $true
    Close-Window $app
    Wait-Visible $app $false
    Start-Sleep -Seconds 1
    if ($app.HasExited) { throw '关闭到托盘应保留进程' }
    $silent = Start-App $true
    if (!$silent.WaitForExit(5000)) { throw '重复 shim 启动未退出' }
    Wait-Visible $app $false
    $second = Start-App $false
    if (!$second.WaitForExit(5000)) { throw '再次启动未退出' }
    Wait-Visible $app $true
    $tray = [TrayRuntimeNative]::TrayWindow()
    [void][TrayRuntimeNative]::PostMessage($tray, 0x0111, [UIntPtr]::new(2), [IntPtr]::Zero)
    if (!$app.WaitForExit(5000)) { throw '托盘退出未结束进程' }
    if ([TrayRuntimeNative]::TrayWindow() -ne [IntPtr]::Zero) { throw '退出后托盘窗口未清理' }
    '{"close_to_tray":false}' | Set-Content (Join-Path $testRoot 'config.json') -Encoding utf8
    $app = Start-App $false
    Wait-Visible $app $true
    Close-Window $app
    if (!$app.WaitForExit(5000)) { throw '默认关闭窗口应退出进程' }
    Write-Output 'PASS: 静默启动、手动唤回、关闭驻留、重复 shim 保持静默、反复恢复、托盘退出、默认关闭退出'
} catch {
    Write-Output $_.ScriptStackTrace
    throw
} finally {
    foreach ($process in $owned) { if (!$process.HasExited) { Stop-Process -Id $process.Id -ErrorAction SilentlyContinue } }
    $env:PACKPORTER_CONFIG_DIR = $oldConfig
    $resolvedRoot = [IO.Path]::GetFullPath($testRoot)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if ($resolvedRoot.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path $resolvedRoot -Leaf).StartsWith('packporter-tray-')) {
        Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
    }
}
