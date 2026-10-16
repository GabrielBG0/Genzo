# Genzo M0 benchmark kit (Windows). The Windows twin of run-bench.sh: records
# the system, runs the cli GPU benchmark, then starts the app once with
# GENZO_AUTOBENCH so it drags its own sliders (WebView2), and packs the results
# into one zip to send back. Double-click run-bench.cmd, or run:
#   powershell -ExecutionPolicy Bypass -File run-bench.ps1
#
# On Windows the gate was measured with real mouse events over DevTools
# (frontend/scripts/measure-latency.mjs); this run uses the autobench's
# synthetic input, so the two can be compared on the same machine.
#
# Written for Windows PowerShell 5.1, which every Windows 10 and 11 has.
param(
  [switch]$NoQuestions,
  [switch]$SkipCli,
  [int]$AppTimeout = 600
)
$ErrorActionPreference = 'Continue'

$Kit = Split-Path -Parent $MyInvocation.MyCommand.Path
$App = Join-Path $Kit 'bin\genzo.exe'
$Cli = Join-Path $Kit 'bin\genzo-cli.exe'
$Ask = -not $NoQuestions
$Stamp = (Get-Date).ToUniversalTime().ToString("yyyyMMdd'T'HHmmss'Z'")
$Commit = 'unknown'
$VersionFile = Join-Path $Kit 'VERSION'
if (Test-Path $VersionFile) {
  $line = Get-Content $VersionFile | Where-Object { $_ -like 'commit *' } | Select-Object -First 1
  if ($line) { $Commit = $line.Substring(7) }
}
$Out = Join-Path $Kit "results-windows-$Commit-$Stamp"
New-Item -ItemType Directory -Force -Path $Out | Out-Null

function Say($text) { Write-Host ''; Write-Host "== $text" }
# Keep the user's folder, user name and computer name out of every file.
function Scrub([string]$text) {
  $text = $text.Replace($env:USERPROFILE, '~')
  if ($env:USERNAME) { $text = $text.Replace($env:USERNAME, 'user') }
  if ($env:COMPUTERNAME) { $text = $text.Replace($env:COMPUTERNAME, 'host') }
  return $text
}
function Save($path, $text) { Set-Content -Path $path -Value (Scrub ($text | Out-String)) -Encoding UTF8 }

# ---------------------------------------------------------------- 1. system
Say "Recording the system ($(Split-Path -Leaf $Out)\system.txt)"
# Files from a download carry the "mark of the web", and Windows warns before
# running an unsigned program that has it. Only this folder is touched.
Get-ChildItem -Path $Kit -Recurse -File | Unblock-File

$os = Get-CimInstance Win32_OperatingSystem
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$ram = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)
$gpus = Get-CimInstance Win32_VideoController |
  Select-Object Name, DriverVersion, @{ n = 'Resolution'; e = { "$($_.CurrentHorizontalResolution)x$($_.CurrentVerticalResolution)" } }, CurrentRefreshRate
$battery = Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue
$webview = $null
foreach ($key in 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
                 'HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}') {
  $v = (Get-ItemProperty -Path $key -Name pv -ErrorAction SilentlyContinue).pv
  if ($v) { $webview = $v; break }
}
$system = @(
  "kit: $((Get-Content $VersionFile -ErrorAction SilentlyContinue) -join ', ')"
  "date: $Stamp"
  "os: $($os.Caption) $($os.Version)"
  "cpu: $($cpu.Name), $($cpu.NumberOfLogicalProcessors) threads"
  "ram_gb: $ram"
  "power: $(if ($battery) { "battery present, status $($battery.BatteryStatus) (2 = on AC)" } else { 'no battery' })"
  "webview2: $webview"
  ''
  '## GPUs and displays'
  ($gpus | Format-Table -AutoSize | Out-String).Trim()
  ''
  '## NVIDIA'
)
if (Get-Command nvidia-smi -ErrorAction SilentlyContinue) {
  $system += (nvidia-smi --query-gpu=name,driver_version,memory.total,pstate --format=csv | Out-String).Trim()
} else { $system += 'no nvidia-smi' }
Save (Join-Path $Out 'system.txt') ($system -join "`r`n")
Get-Content (Join-Path $Out 'system.txt')

# ---------------------------------------------------------------- 2. cli
Say 'GPU benchmark in the command line tool (a few minutes, nothing to do)'
if ($SkipCli) { Write-Host 'skipped' } else {
  foreach ($size in @(@(4032, 3024, '12mp'), @(8064, 6048, '48mp'))) {
    Write-Host "-- $($size[2])"
    $txt = Join-Path $Out "cli-$($size[2]).txt"
    $json = Join-Path $Out "cli-$($size[2]).json"
    $p = Start-Process -FilePath $Cli -ArgumentList 'bench', 'gpu', $size[0], $size[1], 30, '--json', "`"$json`"" `
      -NoNewWindow -PassThru -RedirectStandardOutput "$txt.out" -RedirectStandardError "$txt.err"
    $null = $p.Handle  # without it, ExitCode stays empty after WaitForExit
    if (-not $p.WaitForExit(900000)) { $p.Kill(); $code = 'timeout' } else { $code = $p.ExitCode }
    $text = ((Get-Content "$txt.out", "$txt.err" -ErrorAction SilentlyContinue) -join "`r`n") + "`r`nexit: $code"
    Remove-Item "$txt.out", "$txt.err" -ErrorAction SilentlyContinue
    Save $txt $text
    Get-Content $txt
  }
}

# ---------------------------------------------------------------- 3. app
Say 'App benchmark: 1 run of about a minute'
Write-Host @'
A Genzo window opens and moves its own sliders. While it runs:
  - do not touch the mouse or keyboard,
  - do not minimize, move or cover the window,
  - watch whether the photo in the middle changes brightness and color.
If Windows asks whether to run genzo, choose "Run anyway".
'@
if ($Ask) { Read-Host 'Press Enter to start' | Out-Null }

$name = 'default'
$report = Join-Path $Out "app-$name.json"
$started = Get-Date
$env:GENZO_AUTOBENCH = $report
$p = Start-Process -FilePath $App -PassThru
Remove-Item Env:\GENZO_AUTOBENCH
$status = 'timeout'
while ($true) {
  if (Test-Path $report) { $status = 'ok'; break }
  if ($p.HasExited) { $status = "exited ($($p.ExitCode))"; break }
  if (((Get-Date) - $started).TotalSeconds -ge $AppTimeout) { break }
  Start-Sleep -Seconds 1
}
Start-Sleep -Seconds 1
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
$seen = 'not asked'
if ($Ask) { $seen = Read-Host 'Did the photo visibly change while the sliders moved? [y/n]' }
@{
  kitCommit = $Commit; variant = $name; env = ''; status = $status
  seconds = [int]((Get-Date) - $started).TotalSeconds; imageVisiblyChanged = $seen
} | ConvertTo-Json | Set-Content -Path (Join-Path $Out "app-$name.run.json") -Encoding UTF8
Write-Host "status: $status"

# ---------------------------------------------------------------- 4. pack
Say 'Summary'
function Fmt($s) { if ($null -eq $s.p50) { 'n/a' } else { '{0:N0}/{1:N0}' -f $s.p50, $s.p95 } }
if (Test-Path $report) {
  $r = Get-Content $report -Raw -Encoding UTF8 | ConvertFrom-Json
  Write-Host "app-${name}: engine on $($r.engine.adapter.name)"
  foreach ($c in $r.cases) {
    $zoom = if ($c.zoom -eq 'fit') { 'Fit' } else { '1:1' }
    $head = '  {0,-16} {1,-8} {2,-3}  ' -f $c.source, $c.slider, $zoom
    if ($c.error) { Write-Host "$head$($c.error)" }
    elseif ($c.n -gt 0) {
      $b = $c.breakdown
      Write-Host ('{0}p50 {1:N1} ms, p95 {2:N1} ms, n {3}' -f $head, $c.p50, $c.p95, $c.n)
      Write-Host ("      one request p50/p95: $(Fmt $b.sendToDraw) ms = send to event $(Fmt $b.sendToEvent)" +
        " (engine $(Fmt $b.engine)) + event to draw $(Fmt $b.eventToDraw) (fetch $(Fmt $b.fetch), $($b.fetchKB.p50) KB)")
    } else {
      Write-Host "${head}no samples: $($c.breakdown.engine.n) engine frames, $($c.breakdown.framesDrawn) drawn"
    }
    foreach ($e in $c.pageErrors) { Write-Host "      page error: $e" }
  }
}
$archive = Join-Path $Kit "genzo-bench-results-windows-$Commit-$Stamp.zip"
Compress-Archive -Path $Out -DestinationPath $archive -Force
Say 'Done. Send this file back:'
Write-Host $archive
if ($Ask) { Read-Host 'Press Enter to close' | Out-Null }
