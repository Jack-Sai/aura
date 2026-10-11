param(
    [Parameter(Mandatory = $true)][string]$BundleDir,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Notes,
    [Parameter(Mandatory = $true)][string]$Repo
)

# 从 Tauri 构建产物生成 latest.json（Tauri updater 的更新清单）。
#
# 需要发布到 Release 的三样东西：
#   1. 安装包本体（供用户手动下载）
#   2. .sig 签名文件（updater 校验用）
#   3. latest.json（指向上面两者的下载地址）
#
# 本脚本负责组装 latest.json，不负责上传——上传仍在发版流程里手动完成，
# 因为 latest.json 里的下载地址必须是真实的 Release 资产 URL，
# 而 URL 只有在 Release 创建之后才确定。

$ErrorActionPreference = 'Stop'

$nsis = Get-ChildItem -Path $BundleDir -Filter '*-setup.exe' -Recurse |
    Where-Object { $_.DirectoryName -match 'nsis' } |
    Select-Object -First 1
$msi = Get-ChildItem -Path $BundleDir -Filter '*.msi' -Recurse |
    Select-Object -First 1

if (-not $nsis) { throw "未找到 nsis 安装包，请确认已执行带签名的 tauri build" }

$nsisSigPath = "$($nsis.FullName).sig"
if (-not (Test-Path $nsisSigPath)) { throw "未找到 nsis 签名文件: $nsisSigPath" }
$msiSigPath = if ($msi -and (Test-Path "$($msi.FullName).sig")) { "$($msi.FullName).sig" } else { $null }

# 安装包统一放到 Release 的 Aura_<version>_win_x64_Setup.exe
$releaseName = "Aura_${Version}_win_x64_Setup.exe"
$base = "$Repo/releases/download/v$Version"

$platforms = @(
    @{ architecture = 'x86_64';  url = "$base/$releaseName"; sig = $nsisSigPath }
)
if ($msi) {
    $platforms += @{ architecture = 'x86_64-msi'; url = "$base/Aura_${Version}_win_x64.msi"; sig = $msiSigPath }
}

# Tauri 要求签名字段去掉换行
function Read-Sig($path) {
    if (-not $path) { return '' }
    ((Get-Content -Raw -Encoding UTF8 $path) -replace "`r?`n", '')
}

$platforms = $platforms | ForEach-Object {
    @{ architecture = $_.architecture; url = $_.url; signature = (Read-Sig $_.sig) }
}

$manifest = @{
    version   = $Version
    notes     = $Notes
    pub_date  = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = $platforms
}

$json = $manifest | ConvertTo-Json -Depth 5
$out = Join-Path (Split-Path $BundleDir -Parent) 'latest.json'
[IO.File]::WriteAllText($out, $json, (New-Object Text.UTF8Encoding $false))
Write-Host "已生成 $out"
Write-Host "请在 Release v$Version 中上传：$releaseName 与 latest.json"