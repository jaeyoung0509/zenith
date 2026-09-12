<#
.SYNOPSIS
  Packaging gate for the Windows NSIS installer.

.DESCRIPTION
  Installs the built installer silently, runs `--doctor` against the installed
  binary, fails when any self-check fails or the exit code is not 0, uninstalls
  silently, and fails when the install directory survives. Run by the
  `package-windows` CI job through `just test-package <installer>`.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$InstallerPath,

  [string]$ProductName = 'Zenith',

  [string]$ExecutableName = 'Zenith.exe',

  [int]$TimeoutSeconds = 180
)

$ErrorActionPreference = 'Stop'

function Fail {
  param([string]$Message)
  Write-Host "Error: $Message"
  exit 1
}

# Native commands write progress and warnings to stderr, which PowerShell turns
# into a terminating error while $ErrorActionPreference is 'Stop'. Run them
# with that preference relaxed so only the exit code decides the outcome.
function Invoke-Native {
  param(
    [Parameter(Mandatory = $true)][string]$FilePath,
    [string[]]$Arguments = @()
  )

  $previous = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try {
    $output = & $FilePath @Arguments
    $exitCode = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $previous
  }

  return [pscustomobject]@{ Output = $output; ExitCode = $exitCode }
}

$resolvedInstaller = Resolve-Path -LiteralPath $InstallerPath -ErrorAction SilentlyContinue
if (-not $resolvedInstaller -or -not (Test-Path -LiteralPath $resolvedInstaller -PathType Leaf)) {
  Fail "installer not found: $InstallerPath"
}
$installer = $resolvedInstaller.Path

Write-Host "Installing $installer silently ..."
$installProcess = Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru
if ($installProcess.ExitCode -ne 0) {
  Fail "installer exited with code $($installProcess.ExitCode)"
}

$installRoots = @()
foreach ($base in @($env:LOCALAPPDATA, $env:ProgramFiles, ${env:ProgramFiles(x86)})) {
  if ($base) {
    $installRoots += (Join-Path $base $ProductName)
  }
}

$installedExe = $null
$installRoot = $null
foreach ($root in $installRoots) {
  $candidate = Join-Path $root $ExecutableName
  if (Test-Path -LiteralPath $candidate -PathType Leaf) {
    $installedExe = $candidate
    $installRoot = $root
    break
  }
}

if (-not $installedExe) {
  Fail "installed $ExecutableName was not found under: $($installRoots -join ', ')"
}

Write-Host "Installed application: $installedExe"

Write-Host 'Running --doctor ...'
$doctor = Invoke-Native -FilePath $installedExe -Arguments @('--doctor')
$doctor.Output | ForEach-Object { Write-Host $_ }
if ($doctor.ExitCode -ne 0) {
  Fail "--doctor exited with code $($doctor.ExitCode), so a self-check failed"
}
# Match only a rendered check row. Diagnostic details legitimately use phrases
# such as "fail closed", which must not turn a passing report into a failure.
if ($doctor.Output -match '^\s*\[FAIL\](?:\s|$)') {
  Fail '--doctor printed a FAIL row'
}

$doctorJson = Invoke-Native -FilePath $installedExe -Arguments @('--doctor', '--json')
if ($doctorJson.ExitCode -ne 0) {
  Fail "--doctor --json exited with code $($doctorJson.ExitCode)"
}

try {
  $report = ($doctorJson.Output | Out-String) | ConvertFrom-Json
} catch {
  Fail "--doctor --json did not print parseable JSON: $_"
}

if ($null -eq $report.failures -or $report.failures -ne 0) {
  Fail "doctor report lists $($report.failures) failing self-check(s)"
}

$failed = @($report.checks | Where-Object { $_.outcome -ne 'pass' })
if ($failed.Count -gt 0) {
  Fail "doctor report contains non-passing checks: $($failed.name -join ', ')"
}

Write-Host "Doctor self-check passed for platform '$($report.platform)' with $($report.checks.Count) checks."

$uninstaller = Join-Path $installRoot 'uninstall.exe'
if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
  Fail "uninstaller not found at $uninstaller"
}

Write-Host 'Uninstalling silently ...'
$uninstallProcess = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
if ($uninstallProcess.ExitCode -ne 0) {
  Fail "uninstaller exited with code $($uninstallProcess.ExitCode)"
}

# NSIS uninstallers hand the removal to a temporary copy, so poll until the
# install directory is gone instead of assuming it is removed on exit.
$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
while ((Test-Path -LiteralPath $installRoot) -and (Get-Date) -lt $deadline) {
  Start-Sleep -Milliseconds 500
}

if (Test-Path -LiteralPath $installRoot) {
  Fail "install directory survived uninstall: $installRoot"
}

Write-Host "Packaging smoke test passed: silent install, --doctor self-check, silent uninstall, and no leftover $installRoot."
