param([switch]$DesktopShortcut)
$ErrorActionPreference = 'Stop'
$destination = Join-Path $env:LOCALAPPDATA 'Programs\RPad'
New-Item -ItemType Directory -Force -Path $destination | Out-Null
Copy-Item (Join-Path $PSScriptRoot 'RPad.exe') (Join-Path $destination 'RPad.exe') -Force
foreach ($name in @('LICENSE', 'README.md', 'THIRD_PARTY.md')) {
    Copy-Item (Join-Path $PSScriptRoot $name) $destination -Force
}
if (Test-Path (Join-Path $PSScriptRoot 'licenses')) {
    Copy-Item (Join-Path $PSScriptRoot 'licenses') $destination -Recurse -Force
}
Get-ChildItem $PSScriptRoot -Filter '*.tar.gz' | Copy-Item -Destination $destination -Force
$shell = New-Object -ComObject WScript.Shell
$menu = Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs\RPad.lnk'
$shortcut = $shell.CreateShortcut($menu)
$shortcut.TargetPath = Join-Path $destination 'RPad.exe'
$shortcut.IconLocation = "$destination\RPad.exe,0"
$shortcut.WorkingDirectory = $destination
$shortcut.Save()
if ($DesktopShortcut) {
    $desktop = $shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Desktop')) 'RPad.lnk'))
    $desktop.TargetPath = $shortcut.TargetPath
    $desktop.IconLocation = $shortcut.IconLocation
    $desktop.WorkingDirectory = $destination
    $desktop.Save()
}
Write-Output "Installed RPad in $destination"
