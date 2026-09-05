# 文件职责：用真实 shim 与模拟启动器验证联动会话、派生进程及游戏独立运行。
# 测试只写隔离配置，不创建登录启动项，不修改原启动器。
param([string]$Executable = "$PSScriptRoot/../target/debug/packporter.exe")
$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path -LiteralPath $Executable).Path
$shimPath = Join-Path (Split-Path $exePath) 'packporter-shim.exe'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ("packporter-shim-" + [guid]::NewGuid())
$oldConfigDir = $env:PACKPORTER_CONFIG_DIR
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
function Get-TestWindows { @(Get-Process -Name packporter -ErrorAction SilentlyContinue | Where-Object Path -eq $exePath) }
function Wait-WindowCount([int]$Expected) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        $windows = @(Get-TestWindows)
        if ($windows.Count -eq $Expected) { return }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "窗口进程数量应为 $Expected，实际为 $($windows.Count)"
}
function Start-Shim([string]$Launcher, [string[]]$Forwarded) {
    $info = [Diagnostics.ProcessStartInfo]::new($shimPath)
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    foreach ($arg in (@('--launcher', $Launcher, '--') + $Forwarded)) { $info.ArgumentList.Add($arg) }
    $process = [Diagnostics.Process]::Start($info)
    $owned.Add($process)
    return $process
}
function Wait-Fixture([string]$Marker) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (!(Test-Path -LiteralPath $Marker)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw "启动器未写入标记 $Marker" }
        Start-Sleep -Milliseconds 100
    }
    $lines = Get-Content -LiteralPath $Marker
    $process = Get-Process -Id ([int]$lines[0])
    $owned.Add($process)
    return $process
}
if (@(Get-Process -Name packporter,packporter-shim -ErrorAction SilentlyContinue).Count) {
    throw '请先关闭已有 PackPorter 和 shim，避免实例锁影响测试。'
}
try {
    New-Item -ItemType Directory -Path $testRoot | Out-Null
    $env:PACKPORTER_CONFIG_DIR = $testRoot
    '{"follow_launchers":true}' | Set-Content -LiteralPath (Join-Path $testRoot 'config.json') -Encoding utf8
    $source = Join-Path $testRoot 'fixture.rs'
    @"
use std::{env, fs, process::Command, thread, time::Duration};
fn main() {
    let args: Vec<String> = env::args().collect();
    let value = |key: &str| args.iter().position(|a| a == key).and_then(|i| args.get(i + 1));
    if let Some(marker) = value("--fixture-marker") {
        fs::write(marker, format!("{}\n{}\n{:?}", std::process::id(), env::current_dir().unwrap().display(), args)).unwrap();
    }
    if let Some(child) = value("--fixture-spawn") {
        Command::new(child).args(["-jar", value("--fixture-jar").unwrap(), "--fixture-marker", value("--fixture-child-marker").unwrap()]).spawn().unwrap();
    }
    if args.iter().any(|a| a == "--fixture-handoff") { return; }
    thread::sleep(Duration::from_secs(120));
}
"@ | Set-Content -LiteralPath $source -Encoding utf8
    $launcher = Join-Path $testRoot '重命名 启动器.exe'
    & rustc --crate-name launcher_fixture $source -o $launcher
    if ($LASTEXITCODE -ne 0) { throw '无法编译启动器替身' }
    $java = Join-Path $testRoot 'javaw.exe'
    Copy-Item -LiteralPath $launcher -Destination $java
    $firstMarker = Join-Path $testRoot 'first.txt'
    $secondMarker = Join-Path $testRoot 'second.txt'
    $firstShim = Start-Shim $launcher @('--fixture-marker', $firstMarker, '中文 空格', 'a"b', 'tail\')
    $first = Wait-Fixture $firstMarker
    Wait-WindowCount 1
    $lines = Get-Content -LiteralPath $firstMarker
    # Rust 可返回 Windows 扩展路径前缀；去掉前缀后比较实际目录。
    $workingDirectory = $lines[1] -replace '^\\\\\?\\', ''
    if ($workingDirectory -ne $testRoot) { throw "shim 工作目录为 $($lines[1])，预期 $testRoot" }
    $forwarded = $lines[2] | ConvertFrom-Json
    if ('中文 空格' -notin $forwarded -or 'a"b' -notin $forwarded -or 'tail\' -notin $forwarded) { throw 'shim 参数转发错误' }
    $secondShim = Start-Shim $launcher @('--fixture-marker', $secondMarker)
    $second = Wait-Fixture $secondMarker
    Stop-Process -Id $first.Id
    Start-Sleep -Seconds 2
    if (@(Get-TestWindows).Count -ne 1) { throw '第一个会话退出不应关闭窗口' }
    Stop-Process -Id $second.Id
    Wait-WindowCount 0
    if (!$firstShim.WaitForExit(10000) -or !$secondShim.WaitForExit(10000)) { throw '会话结束后 shim 未退出' }
    # 模拟 HMCL EXE 启动 Java 后自身退出，Java 启动器应继续保持会话。
    $handoffMarker = Join-Path $testRoot 'handoff.txt'
    $handoffShim = Start-Shim $launcher @('--fixture-spawn', $java, '--fixture-jar', 'HMCL.jar', '--fixture-child-marker', $handoffMarker, '--fixture-handoff')
    $handoff = Wait-Fixture $handoffMarker
    Wait-WindowCount 1
    Start-Sleep -Seconds 2
    if (@(Get-TestWindows).Count -ne 1) { throw 'HMCL 转交 Java 后不应关闭窗口' }
    Stop-Process -Id $handoff.Id
    Wait-WindowCount 0
    if (!$handoffShim.WaitForExit(10000)) { throw 'Java 启动器退出后 shim 未退出' }
    # 游戏进程保留在 Job 中，但不能延长启动器会话，更不能被 Job 关闭强制终止。
    $rootMarker = Join-Path $testRoot 'game-root.txt'
    $gameMarker = Join-Path $testRoot 'game.txt'
    $gameShim = Start-Shim $launcher @('--fixture-marker', $rootMarker, '--fixture-spawn', $java, '--fixture-jar', 'minecraft.jar', '--fixture-child-marker', $gameMarker)
    $root = Wait-Fixture $rootMarker
    $game = Wait-Fixture $gameMarker
    Wait-WindowCount 1
    Stop-Process -Id $root.Id
    Wait-WindowCount 0
    if (!$gameShim.WaitForExit(10000)) { throw 'Minecraft 不应延长 shim 会话' }
    if ($game.HasExited) { throw '关闭启动器不应终止 Minecraft' }
    Stop-Process -Id $game.Id
    # 禁用时快捷方式仍应正常启动原启动器，但不创建联动 UI 或常驻 shim。
    '{"follow_launchers":false}' | Set-Content -LiteralPath (Join-Path $testRoot 'config.json') -Encoding utf8
    $disabledMarker = Join-Path $testRoot 'disabled.txt'
    $disabledShim = Start-Shim $launcher @('--fixture-marker', $disabledMarker)
    $disabled = Wait-Fixture $disabledMarker
    if (!$disabledShim.WaitForExit(10000)) { throw '禁用联动后 shim 不应驻留' }
    Wait-WindowCount 0
    if ($disabled.HasExited) { throw '禁用联动不应阻止启动器运行' }
    Write-Output 'PASS: 多会话最后退出、参数与工作目录、HMCL Java 转交、游戏独立运行、禁用无常驻'
} catch {
    Write-Output $_.ScriptStackTrace
    throw
} finally {
    foreach ($process in $owned) {
        if (!$process.HasExited) { Stop-Process -Id $process.Id -ErrorAction SilentlyContinue }
    }
    Get-TestWindows | Stop-Process -ErrorAction SilentlyContinue
    $env:PACKPORTER_CONFIG_DIR = $oldConfigDir
    $resolvedRoot = [IO.Path]::GetFullPath($testRoot)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if ($resolvedRoot.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path $resolvedRoot -Leaf).StartsWith('packporter-shim-')) {
        Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
    }
}
