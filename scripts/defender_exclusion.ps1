<#
.SYNOPSIS
    Adds (or removes) Windows Defender exclusions for the LawfulOverlay client.

.DESCRIPTION
    LawfulOverlay.exe is unsigned and replaces itself when it updates, which
    can make Defender flag or quarantine it. This script excludes:
      - the exe itself                      (path + process)
      - <exe name>.new                      file the auto-updater downloads
      - <exe name>.old                      previous exe, moved aside during an update
      - the settings folder                 %USERPROFILE%\.discord_chat_overlay
                                            (or the folder of LAWFUL_OVERLAY_CONFIG)

    Every path is worked out on the machine that runs the script, so it works
    for any user and any install folder. Run it again after you move the exe.

.PARAMETER ExePath
    Full path to LawfulOverlay.exe. Default: LawfulOverlay.exe next to this
    script, else ..\dist\LawfulOverlay.exe (the repo build output).

.PARAMETER Remove
    Remove the exclusions instead of adding them.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\defender_exclusion.ps1
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\defender_exclusion.ps1 -ExePath "D:\Apps\LawfulOverlay.exe"
.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\defender_exclusion.ps1 -Remove
#>
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$Remove,
    [Parameter(DontShow)][switch]$PauseAtEnd   # set on the elevated relaunch
)

$ErrorActionPreference = 'Stop'

# --- Resolve the exe path (relative to this script, never hard-coded) -------
if (-not $ExePath) {
    $candidates = @(
        (Join-Path $PSScriptRoot 'LawfulOverlay.exe'),
        (Join-Path (Split-Path $PSScriptRoot -Parent) 'dist\LawfulOverlay.exe')
    )
    $ExePath = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if (-not $ExePath) {
        if ($Remove) {
            # Removing still makes sense if the exe is gone; fall back to the first guess.
            $ExePath = $candidates[0]
        } else {
            Write-Host "[ERROR] LawfulOverlay.exe not found. Looked in:" -ForegroundColor Red
            $candidates | ForEach-Object { Write-Host "        $_" }
            Write-Host "        Pass the path: -ExePath 'C:\path\to\LawfulOverlay.exe'"
            exit 1
        }
    }
}
$ExePath = [System.IO.Path]::GetFullPath($ExePath)

# --- Relaunch elevated if needed --------------------------------------------
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host "[INFO] Administrator rights are needed. Asking Windows to relaunch elevated ..."
    $hostExe = (Get-Process -Id $PID).Path
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-ExePath', "`"$ExePath`"", '-PauseAtEnd')
    if ($Remove) { $argList += '-Remove' }
    try {
        $p = Start-Process -FilePath $hostExe -ArgumentList $argList -Verb RunAs -Wait -PassThru
        exit $p.ExitCode
    } catch {
        Write-Host "[ERROR] Elevation was cancelled or failed. Right-click PowerShell -> 'Run as administrator' and run this script again." -ForegroundColor Red
        exit 1
    }
}

if (-not (Get-Command Add-MpPreference -ErrorAction SilentlyContinue)) {
    Write-Host "[ERROR] The Defender cmdlets (Add-MpPreference) are not available on this system." -ForegroundColor Red
    exit 1
}

# --- What to exclude ---------------------------------------------------------
$exeDir  = Split-Path $ExePath -Parent
$exeBase = [System.IO.Path]::GetFileNameWithoutExtension($ExePath)

if ($env:LAWFUL_OVERLAY_CONFIG) {
    $configDir = Split-Path ([System.IO.Path]::GetFullPath($env:LAWFUL_OVERLAY_CONFIG)) -Parent
} else {
    $configDir = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.discord_chat_overlay'
}

$paths = @(
    $ExePath,
    (Join-Path $exeDir "$exeBase.new"),   # updater download
    (Join-Path $exeDir "$exeBase.old"),   # previous exe during the swap
    $configDir
)
$processes = @($ExePath)

# --- Apply -------------------------------------------------------------------
function Test-Listed([object[]]$List, [string]$Item) {
    foreach ($x in @($List)) { if ($x -and $x.TrimEnd('\') -ieq $Item.TrimEnd('\')) { return $true } }
    return $false
}

$pref = Get-MpPreference
$changed = 0

foreach ($p in $paths) {
    $listed = Test-Listed $pref.ExclusionPath $p
    if ($Remove) {
        if ($listed) { Remove-MpPreference -ExclusionPath $p; Write-Host "[REMOVED] path    $p" -ForegroundColor Yellow; $changed++ }
        else         { Write-Host "[SKIP]    path    $p (not excluded)" }
    } else {
        if (-not $listed) { Add-MpPreference -ExclusionPath $p; Write-Host "[ADDED]   path    $p" -ForegroundColor Green; $changed++ }
        else              { Write-Host "[SKIP]    path    $p (already excluded)" }
    }
}

foreach ($p in $processes) {
    $listed = Test-Listed $pref.ExclusionProcess $p
    if ($Remove) {
        if ($listed) { Remove-MpPreference -ExclusionProcess $p; Write-Host "[REMOVED] process $p" -ForegroundColor Yellow; $changed++ }
        else         { Write-Host "[SKIP]    process $p (not excluded)" }
    } else {
        if (-not $listed) { Add-MpPreference -ExclusionProcess $p; Write-Host "[ADDED]   process $p" -ForegroundColor Green; $changed++ }
        else              { Write-Host "[SKIP]    process $p (already excluded)" }
    }
}

$verb = if ($Remove) { 'removed' } else { 'added' }
Write-Host ""
Write-Host "[OK] $changed exclusion(s) $verb."
if ($PauseAtEnd) { Read-Host "Press Enter to close" | Out-Null }   # keep the elevated window readable
