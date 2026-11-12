# Measures Snowie's resource usage: CPU (Snowie and DWM, which composes the overlay),
# memory, GPU load and GPU memory. Usage:
#   powershell -ExecutionPolicy Bypass -File scripts\measure.ps1 [-Exe path] [-Seconds 20] [-Env "SNOWIE_PRESNOW=1,SNOWIE_LOG=1"]
#   -Baseline measures the system without Snowie.
param(
    [string]$Exe = "$PSScriptRoot\..\target\release\snowie.exe",
    [int]$Seconds = 20,
    [int]$Settle = 6,
    [string]$Env = "",
    [switch]$Music,
    [switch]$Baseline
)

$cores = (Get-CimInstance Win32_Processor | Measure-Object NumberOfLogicalProcessors -Sum).Sum
Get-Process snowie -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep 1

$proc = $null
if (-not $Baseline) {
    # isolated default settings, so results do not depend on the user's config
    $appdata = Join-Path $env:TEMP "snowie-measure"
    New-Item -ItemType Directory -Force (Join-Path $appdata "Snowie") | Out-Null
    $musicJson = if ($Music) { 'true' } else { 'false' }
    Set-Content (Join-Path $appdata "Snowie\config.json") "{ `"music`": $musicJson }" -Encoding ascii
    $pairs = @("APPDATA=$appdata") + ($Env -split '[,;]' | Where-Object { $_ })
    $saved = @{}
    foreach ($e in $pairs) { $k, $v = $e -split '=', 2; $saved[$k] = [Environment]::GetEnvironmentVariable($k); Set-Item "env:$k" $v }
    $proc = Start-Process $Exe -PassThru
    foreach ($k in $saved.Keys) { if ($null -eq $saved[$k]) { Remove-Item "env:$k" } else { Set-Item "env:$k" $saved[$k] } }
}
Start-Sleep $Settle

# DWM is a protected process, so it is read through counters; GPU load is the total over all processes
$paths = @('\Process(dwm)\% Processor Time', '\GPU Engine(*engtype_3D)\Utilization Percentage')
if ($proc) { $paths += "\GPU Process Memory(pid_$($proc.Id)_*)\Dedicated Usage" }

$t0 = if ($proc) { $proc.Refresh(); $proc.TotalProcessorTime.TotalSeconds } else { 0 }
$sw = [Diagnostics.Stopwatch]::StartNew()
$samples = Get-Counter -Counter $paths -SampleInterval 1 -MaxSamples $Seconds -ErrorAction SilentlyContinue
$el = $sw.Elapsed.TotalSeconds

$gpuSnow = 0; $gpuTotal = 0; $dwmCpu = 0; $gpuMem = 0; $n = [math]::Max(1, $samples.Count)
foreach ($s in $samples) {
    foreach ($cs in $s.CounterSamples) {
        if ($cs.Path -like '*process(dwm)*') { $dwmCpu += $cs.CookedValue }
        elseif ($cs.Path -like '*dedicated usage*') { $gpuMem = [math]::Max($gpuMem, $cs.CookedValue) }
        elseif ($cs.Path -like '*utilization*') {
            $gpuTotal += $cs.CookedValue
            if ($proc -and $cs.Path -like "*pid_$($proc.Id)_*") { $gpuSnow += $cs.CookedValue }
        }
    }
}
$r = [ordered]@{}
if ($proc) {
    $proc.Refresh()
    $cpu = ($proc.TotalProcessorTime.TotalSeconds - $t0) / $el * 100
    $r['Snowie CPU, % of 1 core'] = [math]::Round($cpu, 1)
    $r["Snowie CPU, % of all $cores threads"] = [math]::Round($cpu / $cores, 2)
    $r['Snowie RAM working set, MB'] = [math]::Round($proc.WorkingSet64 / 1MB, 1)
    $r['Snowie RAM private, MB'] = [math]::Round($proc.PrivateMemorySize64 / 1MB, 1)
    $r['Snowie GPU 3D, %'] = [math]::Round($gpuSnow / $n, 1)
    $r['Snowie GPU memory, MB'] = [math]::Round($gpuMem / 1MB, 1)
}
$r['DWM CPU, % of 1 core'] = [math]::Round($dwmCpu / $n, 1)
$r['GPU 3D total (all apps), %'] = [math]::Round($gpuTotal / $n, 1)
[pscustomobject]$r | Format-List | Out-String
if ($proc) { Stop-Process -Id $proc.Id -Force }
