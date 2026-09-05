# 文件职责：UI 接线契约检查脚本。
# 检查目标：ui/packporter.slint 中每个可交互控件必须连接到对应回调，
#           Rust 侧必须实现全部回调处理器——修复"按钮/下拉菜单点击无反应"缺陷；
#           并锁定无边框窗口镶边的关键接线（no-frame / 窗口控制按钮 / 镶边安装）。
# 用法：pwsh -File tests/check_ui_wiring.ps1（需 PowerShell 7，见经验 3：UTF-8 无 BOM 脚本）
# 退出码：0 = 契约满足（绿）；1 = 存在未接线控件（红）。

$ErrorActionPreference = 'Stop'
$slintPath = Join-Path $PSScriptRoot '..\ui\packporter.slint'
$mainPath  = Join-Path $PSScriptRoot '..\src\main.rs'
$slint = Get-Content $slintPath -Raw
$main  = Get-Content $mainPath -Raw

$failures = @()

# 契约 1：固定文案按钮的 clicked 处理器必须调用对应回调（而非空实现）。
$buttonWiring = @(
    @{ Text = '开始迁移';     Callback = 'execute-requested' },
    @{ Text = '打开备份目录'; Callback = 'open-backup-folder' }
)
foreach ($w in $buttonWiring) {
    # 定位指定文案的 Button 块，要求其 clicked => root.<回调>() 存在。
    $pattern = '(?s)Button\s*\{[^}]*text:\s*"' + [regex]::Escape($w.Text) + '"[^}]*clicked\s*=>\s*\{\s*root\.' + [regex]::Escape($w.Callback) + '\(\);'
    if ($slint -notmatch $pattern) {
        $failures += "按钮「$($w.Text)」未接线到回调 root.$($w.Callback)"
    }
}

# 契约 1b：文案动态的按钮（三元表达式）按回调名锚定：clicked 中必须出现对应回调调用。
$dynamicWiring = @(
    @{ Name = '扫描实例/重新扫描'; Callback = 'scan-requested' },
    @{ Name = '重新生成计划';     Callback = 'plan-requested' }
)
foreach ($w in $dynamicWiring) {
    if ($slint -notmatch ('clicked\s*=>\s*\{\s*root\.' + [regex]::Escape($w.Callback) + '\(\);')) {
        $failures += "按钮「$($w.Name)」未接线到回调 root.$($w.Callback)"
    }
}

# 契约 2：两个实例下拉框必须绑定模型，且选中索引与 root 属性双向同步。
if ($slint -notmatch '(?s)current-index\s*<=>\s*root\.source-index') {
    $failures += '源实例下拉框未双向绑定 root.source-index'
}
if ($slint -notmatch '(?s)current-index\s*<=>\s*root\.target-index') {
    $failures += '目标实例下拉框未双向绑定 root.target-index'
}
if (([regex]::Matches($slint, 'model:\s*root\.instance-names')).Count -lt 2) {
    $failures += '两个实例下拉框均需绑定 instance-names 模型'
}

# 契约 3：Rust 侧（库层控制器）必须注册全部四个回调处理器。
$srcText = (Get-ChildItem (Join-Path $PSScriptRoot '..\src') -Filter '*.rs' |
    Get-Content -Raw) -join "`n"
foreach ($cb in @('on_scan_requested', 'on_plan_requested', 'on_execute_requested', 'on_open_backup_folder')) {
    if ($srcText -notmatch [regex]::Escape($cb)) {
        $failures += "Rust 侧缺少回调处理器 $cb"
    }
}

# 契约 4：无边框窗口镶边：去系统边框、三个窗口控制按钮接线、入口安装镶边。
if ($slint -notmatch 'no-frame:\s*true') {
    $failures += '窗口缺少 no-frame: true（无边框镶线前提）'
}
$windowControlWiring = @(
    @{ Name = '最小化';      Action = 'root\.minimized\s*=\s*true' },
    @{ Name = '最大化/还原'; Action = 'root\.maximized\s*=\s*!\s*root\.maximized' },
    @{ Name = '关闭';        Action = 'root\.close\(\)' }
)
foreach ($w in $windowControlWiring) {
    $pattern = '(?s)WindowControlButton\s*\{[^}]*clicked\s*=>\s*\{\s*' + $w.Action
    if ($slint -notmatch $pattern) {
        $failures += "窗口控制按钮「$($w.Name)」未接线（期望 $($w.Action)）"
    }
}
if ($slint -notmatch 'titlebar-controls-x:\s*titlebar-controls\.x') {
    $failures += '标题栏控制区起点未暴露给命中测试（titlebar-controls-x）'
}
if ($main -notmatch 'install_frameless_chrome') {
    $failures += '入口未安装无边框窗口镶边（install_frameless_chrome）'
}

# 契约 5：设置齿轮为开合开关：点击处理器按 settings-open 在打开/取消回调间切换。
if ($slint -notmatch '(?s)titlebar-gear\s*:=\s*GearButton\s*\{.{0,400}?root\.settings-open.{0,200}?root\.cancel-settings\(\).{0,200}?root\.open-settings\(\)') {
    $failures += '设置齿轮未按 settings-open 切换 open/cancel 回调（开合开关）'
}

if ($failures.Count -gt 0) {
    Write-Host 'UI 接线契约检查未通过：'
    $failures | ForEach-Object { Write-Host "  - $_" }
    exit 1
}
Write-Host 'UI 接线契约检查通过：固定与动态按钮、下拉框、Rust 回调处理器与无边框镶边全部接线。'
exit 0
