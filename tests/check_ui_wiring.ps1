# 文件职责：UI 接线契约检查脚本。
# 检查目标：ui/packporter.slint 中每个可交互控件必须连接到对应回调，
#           Rust 侧必须实现全部回调处理器——修复"按钮/下拉菜单点击无反应"缺陷。
# 用法：powershell -File tests/check_ui_wiring.ps1
# 退出码：0 = 契约满足（绿）；1 = 存在未接线控件（红）。

$ErrorActionPreference = 'Stop'
$slintPath = Join-Path $PSScriptRoot '..\ui\packporter.slint'
$mainPath  = Join-Path $PSScriptRoot '..\src\main.rs'
$slint = Get-Content $slintPath -Raw
$main  = Get-Content $mainPath -Raw

$failures = @()

# 契约 1：每个按钮的 clicked 处理器必须调用对应回调（而非空实现）。
$buttonWiring = @(
    @{ Text = '扫描实例';     Callback = 'scan-requested' },
    @{ Text = '生成迁移计划'; Callback = 'plan-requested' },
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

if ($failures.Count -gt 0) {
    Write-Host 'UI 接线契约检查未通过：'
    $failures | ForEach-Object { Write-Host "  - $_" }
    exit 1
}
Write-Host 'UI 接线契约检查通过：4 个按钮、2 个下拉框、4 个 Rust 回调处理器全部接线。'
exit 0
