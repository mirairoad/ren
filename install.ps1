$ErrorActionPreference = 'Stop'
$repo = 'mirairoad/ren'
$dir = Join-Path $env:LOCALAPPDATA 'Programs\ren'
$exe = Join-Path $dir 'ren.exe'

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
New-Item -ItemType Directory -Force -Path $dir | Out-Null

Get-Process ren -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force

Write-Host 'Downloading ren...'
$url = "https://github.com/$repo/releases/latest/download/ren-windows-x86_64.exe"
Invoke-WebRequest -Uri $url -OutFile $exe -UseBasicParsing
& $exe --init

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not $userPath) { $userPath = '' }
if (-not (($userPath -split ';') -contains $dir)) {
    [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ';' + $dir).TrimStart(';'), 'User')
}

$shell = New-Object -ComObject WScript.Shell
$wt = Get-Command wt.exe -ErrorAction SilentlyContinue
$places = @(
    [Environment]::GetFolderPath('Desktop'),
    [Environment]::GetFolderPath('Programs')
)
foreach ($p in $places) {
    $lnk = $shell.CreateShortcut((Join-Path $p 'Ren IDE.lnk'))
    if ($wt) {
        $lnk.TargetPath = $wt.Source
        $lnk.Arguments = "--title `"Ren IDE`" `"$exe`""
    } else {
        $lnk.TargetPath = $exe
    }
    $lnk.IconLocation = "$exe,0"
    $lnk.WorkingDirectory = $env:USERPROFILE
    $lnk.Description = 'Ren IDE'
    $lnk.Save()
}

Write-Host ''
Write-Host 'ren installed.'
Write-Host '  Desktop shortcut: Ren IDE'
Write-Host '  Terminal command: ren  (open a new terminal first)'
