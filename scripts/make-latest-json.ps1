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

# **必须按版本号过滤**。
#
# bundle/ 目录里会残留上一次的构建产物（0.5.0 的 exe/msi/sig 都在），
# 而 `Get-ChildItem` 的返回顺序不保证最新在前。此前用
# `Select-Object -First 1` 直接取，v0.5.1 发版时选中的却是 **0.5.0 的
# .sig**——manifest 里的 signature 与实际安装包不匹配，
# updater 的签名校验必然失败，而且是**静默失败**：检查更新一切正常，
# 点下载安装时才炸。
$nsis = Get-ChildItem -Path $BundleDir -Filter '*-setup.exe' -Recurse |
    Where-Object { $_.DirectoryName -match 'nsis' -and $_.BaseName -like "*_$Version*" } |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
$msi = Get-ChildItem -Path $BundleDir -Filter '*.msi' -Recurse |
    Where-Object { $_.BaseName -like "*_$Version*" } |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1

if (-not $nsis) { throw "未找到 $Version 的 nsis 安装包，请确认已执行带签名的 tauri build" }

$nsisSigPath = "$($nsis.FullName).sig"
if (-not (Test-Path $nsisSigPath)) { throw "未找到 nsis 签名文件: $nsisSigPath" }
$msiSigPath = if ($msi -and (Test-Path "$($msi.FullName).sig")) { "$($msi.FullName).sig" } else { $null }

# MSI 存在却没有签名文件时**必须报错**：manifest 里会出现一个 signature
# 为空的条目，客户端选中 msi 架构时签名校验直接失败。
if ($msi -and -not $msiSigPath) {
    throw "找到 $Version 的 MSI 但缺少 .sig：$($msi.FullName).sig"
}

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
    $sig = Read-Sig $_.sig
    if (-not $sig) {
        throw "签名为空，拒绝生成 manifest：$($_.sig)"
    }
    @{ architecture = $_.architecture; url = $_.url; signature = $sig }
}

# 自检：manifest 里的签名必须与**本次实际选中**的安装包配套。
# minisign 每次构建都不同（签名含随机数），因此签名与文件的绑定关系
# 只有在这里能确认。历史上出现过「URL 指向 0.5.1、签名取自 0.5.0」的
# 事故——发布后一切正常，直到用户在客户端点更新才失败。
foreach ($p in $platforms) {
    $local = $p.url.Split('/')[-1]
    $src = if ($p.architecture -eq 'x86_64') { $nsis } else { $msi }
    if (-not $src) { continue }
    $expected = if ($p.architecture -eq 'x86_64') {
        ((Get-Content -Raw -Encoding UTF8 $nsisSigPath) -replace "`r?`n", '')
    } else {
        ((Get-Content -Raw -Encoding UTF8 $msiSigPath) -replace "`r?`n", '')
    }
    if ($p.signature -ne $expected) {
        throw "签名与安装包不匹配：$local（$($src.Name)）"
    }
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
Write-Host "  选中安装包：$($nsis.Name)"
if ($msi) { Write-Host "  选中 MSI ：$($msi.Name)" }
Write-Host "请在 Release v$Version 中上传：$releaseName、Aura_${Version}_win_x64.msi 与 latest.json"