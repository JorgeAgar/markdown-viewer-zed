param([Parameter(Mandatory=$true)][string]$Installer)
$ErrorActionPreference = 'Stop'
$InstallDir = Join-Path $env:RUNNER_TEMP 'mdview-install-test'
$setup = Start-Process -FilePath $Installer -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/DIR=`"$InstallDir`"") -Wait -PassThru
if ($setup.ExitCode -ne 0) { throw "Installer failed: $($setup.ExitCode)" }
$exe = Join-Path $InstallDir 'mdview.exe'
# Redirecting output also exercises the GUI-subsystem executable's CLI support.
$p = Start-Process $exe -ArgumentList '--help' -RedirectStandardOutput "$InstallDir/help.txt" -Wait -PassThru
if ($p.ExitCode -ne 0 -or !(Select-String 'USAGE' "$InstallDir/help.txt" -Quiet)) { throw 'Installed help smoke test failed' }
# Install over the existing installation to exercise the upgrade path.
$p = Start-Process -FilePath $Installer -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/DIR=`"$InstallDir`"") -Wait -PassThru
if ($p.ExitCode -ne 0) { throw 'Upgrade smoke test failed' }
Remove-Item "$InstallDir/help.txt"
$p = Start-Process (Join-Path $InstallDir 'unins000.exe') -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') -Wait -PassThru
if ($p.ExitCode -ne 0 -or (Test-Path $exe)) { throw 'Uninstall smoke test failed' }
