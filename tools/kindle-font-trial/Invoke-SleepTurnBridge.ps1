[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^/root/weread-[A-Za-z0-9_-]+\.[A-Za-z0-9]+$')][string]$Worker,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$')][string]$PlanName,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{16}$')][string]$Label,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9][A-Za-z0-9_.:-]{0,95}$')][string]$Owner,
    [Parameter(Mandatory)][ValidatePattern('^/root/weread-[A-Za-z0-9_-]+\.[A-Za-z0-9]+/device_lease\.py$')][string]$LeaseTool,
    [Parameter(Mandatory)][System.Security.SecureString]$DevicePassword,
    [Parameter(Mandatory)][ValidatePattern('^[\p{L}\p{N}\p{Zs}\uFF0C\u3002\uFF1B\uFF1A\u3001\uFF01\uFF1F,.!?-]{1,18}$')][string]$NoticeTitle,
    [Parameter(Mandatory)][ValidatePattern('^[\p{L}\p{N}\p{Zs}\uFF0C\u3002\uFF1B\uFF1A\u3001\uFF01\uFF1F,.!?-]{1,28}$')][string]$SleepNotice,
    [Parameter(Mandatory)][ValidatePattern('^[\p{L}\p{N}\p{Zs}\uFF0C\u3002\uFF1B\uFF1A\u3001\uFF01\uFF1F,.!?-]{1,28}$')][string]$TurnNotice,
    [Parameter(Mandatory)][ValidatePattern('^[\p{L}\p{N}\p{Zs}\uFF0C\u3002\uFF1B\uFF1A\u3001\uFF01\uFF1F,.!?-]{1,28}$')][string]$WakeNotice,
    [Parameter(Mandatory)][ValidatePattern('^[\p{L}\p{N}\p{Zs}\uFF0C\u3002\uFF1B\uFF1A\u3001\uFF01\uFF1F,.!?-]{1,28}$')][string]$RecoveryNotice,
    [switch]$OperatorReady
)

$ErrorActionPreference = 'Stop'
if ($Owner.StartsWith('-') -or $Label -cnotmatch '^[0-9a-f]{16}$') { throw 'Invalid operator visit identity.' }
if (-not $OperatorReady -or $DevicePassword.Length -eq 0 -or [Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
    -not [Environment]::UserInteractive -or [Diagnostics.Process]::GetCurrentProcess().SessionId -eq 0) {
    throw 'Require a present operator and an interactive Windows desktop before requesting a lease.'
}
$null = Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'show-operator-notice.ps1')
$noticeHost = (Get-Command powershell.exe -CommandType Application).Source
$noticePath = Join-Path $PSScriptRoot 'show-operator-notice.ps1'
$clock = [Diagnostics.Stopwatch]::StartNew()
$messages = @{
    sleep = $SleepNotice
    turn = $TurnNotice
    wake = $WakeNotice
    'recover-wake' = $RecoveryNotice
}
$sshOptions = @('-T','-o','BatchMode=yes','-o','ConnectTimeout=6','test-env')
$runner = "$Worker/src/tools/kindle-font-trial/run_assisted_sleep.py"
$plan = "$Worker/$PlanName"
$base = "python3 -B $runner --work $Worker --plan $plan"
$check = & ssh @sshOptions "$base --preflight"
if ($LASTEXITCODE -ne 0) { throw 'Worker preflight failed. No visit started.' }
$check = ($check -join "`n") | ConvertFrom-Json
if ($check.label -cne $Label -or $check.validated -ne $true -or $check.device_accessed -ne $false) {
    throw 'Frozen plan identity mismatch. No visit started.'
}
$beforeClock = $clock.ElapsedMilliseconds
$clockLine = & ssh @sshOptions 'date +%s%3N'
$afterClock = $clock.ElapsedMilliseconds
if ($LASTEXITCODE -ne 0 -or "$clockLine" -notmatch '^\d{13}$' -or $afterClock - $beforeClock -ge 2000) {
    throw 'The worker clock could not be bounded. No visit started.'
}
$workerTime = [long]$clockLine
$process = [Diagnostics.Process]::new()
$process.StartInfo.FileName = (Get-Command ssh -CommandType Application).Source
$process.StartInfo.Arguments = "-T -o BatchMode=yes -o ConnectTimeout=6 test-env `"$base --lease-tool $LeaseTool --owner $Owner --operator-ready`""
$process.StartInfo.UseShellExecute = $false
$process.StartInfo.CreateNoWindow = $true
$process.StartInfo.RedirectStandardInput = $true
$process.StartInfo.RedirectStandardOutput = $true
if ($process.StartInfo.PSObject.Properties['StandardInputEncoding']) {
    $process.StartInfo.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
}
$popup = $null
$started = $false
$delivered = [Collections.Generic.HashSet[string]]::new()
try {
    $started = $process.Start()
    if (-not $started) { throw 'Unable to start worker transport.' }
    $secret = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($DevicePassword)
    try { $process.StandardInput.WriteLine([Runtime.InteropServices.Marshal]::PtrToStringBSTR($secret)) }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($secret); $process.StandardInput.Close() }
    while ($null -ne ($line = $process.StandardOutput.ReadLine())) {
        if ($line -notmatch '^SLEEP_CUE ([0-9a-f]{16}) (sleep|turn|wake|recover-wake) (\d{13})$') {
            Write-Output $line
            continue
        }
        $cueLabel = $Matches[1]; $phase = $Matches[2]; $issued = [long]$Matches[3]
        $now = $clock.ElapsedMilliseconds
        $ageMin = $workerTime + $now - $afterClock - $issued
        $ageMax = $workerTime + $now - $beforeClock - $issued
        $ordered = switch ($phase) {
            sleep { $delivered.Count -eq 0 }
            turn { $delivered.SetEquals([string[]]@('sleep')) }
            wake { $delivered.SetEquals([string[]]@('sleep','turn')) }
            'recover-wake' { $delivered.Contains('sleep') -and -not $delivered.Contains('wake') }
        }
        if ($cueLabel -cne $Label -or $ageMax -lt 0 -or $ageMin -gt 2000 -or $ageMax -gt 2000 -or
            -not $ordered -or $delivered.Contains($phase) -or $delivered.Contains('recover-wake')) {
            Write-Output 'Suppressed an expired or out-of-order notice.'
            continue
        }
        if ($popup -and -not $popup.HasExited) { $popup.Kill(); $popup.WaitForExit() }
        if ($popup) { $popup.Dispose() }
        $expiry = [Math]::Max(1, [Math]::Floor((8000 - $ageMax) / 1000))
        $popup = Start-Process -FilePath $noticeHost -WindowStyle Hidden -PassThru -ArgumentList @(
            '-NoProfile','-NonInteractive','-File',('"' + $noticePath + '"'),'-Title',('"' + $NoticeTitle + '"'),
            '-Message',('"' + $messages[$phase] + '"'),'-TimeoutSeconds',$expiry
        )
        $null = $delivered.Add($phase)
        Write-Output "Live operator notice dispatched: $phase"
    }
    $process.WaitForExit()
    $exitCode = $process.ExitCode
} finally {
    if ($popup -and -not $popup.HasExited) { $popup.Kill(); $popup.WaitForExit() }
    if ($popup) { $popup.Dispose() }
    if ($started -and -not $process.HasExited) {
        [Console]::Error.WriteLine('Keep the device still. Original worker attempt may require close-only recovery; do not repeat a power action.')
    }
    $process.Dispose()
}
exit $exitCode
