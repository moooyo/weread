[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^/root/weread-[A-Za-z0-9_-]+\.[A-Za-z0-9]+$')][string]$Worker,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$')][string]$PlanName,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{16}$')][string]$Label,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9][A-Za-z0-9_.:-]{0,95}$')][string]$Owner,
    [Parameter(Mandatory)][ValidatePattern('^/root/weread-[A-Za-z0-9_-]+\.[A-Za-z0-9]+/device_lease\.py$')][string]$LeaseTool,
    [Parameter(Mandatory)][System.Security.SecureString]$DevicePassword,
    [Parameter(Mandatory)][ValidatePattern('^[\p{L}\p{N}\p{Zs}\uFF0C\u3002\uFF1B\uFF1A\u3001\uFF01\uFF1F,.!?-]{1,18}$')][string]$NoticeTitle,
    [Parameter(Mandatory)][ValidatePattern('^[\p{L}\p{N}\p{Zs}\uFF0C\u3002\uFF1B\uFF1A\u3001\uFF01\uFF1F,.!?-]{1,28}$')][string]$ReadinessNotice,
    [Parameter(Mandatory)][ValidatePattern('^[\p{L}\p{N}\p{Zs}\uFF0C\u3002\uFF1B\uFF1A\u3001\uFF01\uFF1F,.!?-]{1,28}$')][string]$TurnNotice,
    [switch]$OperatorReady,
    [switch]$PeerHandoffConfirmed
)

$ErrorActionPreference = 'Stop'
if (-not $OperatorReady -or -not $PeerHandoffConfirmed -or $Label -cnotmatch '^[0-9a-f]{16}$' -or
    $DevicePassword.Length -eq 0 -or [Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
    -not [Environment]::UserInteractive -or [Diagnostics.Process]::GetCurrentProcess().SessionId -eq 0) {
    throw 'Require a present operator, peer handoff and an interactive Windows desktop. No visit started.'
}
$noticeHost = (Get-Command powershell.exe -CommandType Application).Source
$noticePath = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'show-operator-notice.ps1')).Path
$clock = [Diagnostics.Stopwatch]::StartNew()

function Confirm-ShortNotice($text, $seconds, $transport, $pendingOutput) {
    $popup = [Diagnostics.Process]::new()
    $popup.StartInfo.FileName = $noticeHost
    $popup.StartInfo.Arguments = "-NoProfile -NonInteractive -File `"$noticePath`" -Title `"$NoticeTitle`" -Message `"$text`" -TimeoutSeconds $seconds -Confirm"
    $popup.StartInfo.UseShellExecute = $false
    $popup.StartInfo.CreateNoWindow = $true
    $popup.StartInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $popup.StartInfo.RedirectStandardOutput = $true
    $opened = $false
    try {
        $opened = $popup.Start()
        if (-not $opened) { return $false }
        $since = $clock.ElapsedMilliseconds
        while (-not $popup.WaitForExit(100)) {
            if (($transport -and ($transport.HasExited -or $pendingOutput.IsCompleted)) -or
                $clock.ElapsedMilliseconds - $since -ge ($seconds + 1) * 1000) { return $false }
        }
        if ($popup.ExitCode -ne 0 -or $popup.StandardOutput.ReadToEnd().TrimEnd("`r", "`n") -cne 'ack') { return $false }
        while ($transport -and $clock.ElapsedMilliseconds - $since -lt 5000) {
            if ($transport.HasExited -or $pendingOutput.IsCompleted) { return $false }
            Start-Sleep -Milliseconds 100
        }
        return (-not $transport -or (-not $transport.HasExited -and -not $pendingOutput.IsCompleted))
    } finally {
        if ($opened -and -not $popup.HasExited) { $popup.Kill(); $popup.WaitForExit() }
        $popup.Dispose()
    }
}

$sshOptions = @('-T','-o','BatchMode=yes','-o','ConnectTimeout=6','test-env')
$base = "python3 -B $Worker/src/tools/kindle-font-trial/run_pose_flip.py --work $Worker --plan $Worker/$PlanName"
$preflight = & ssh @sshOptions "$base --preflight"
if ($LASTEXITCODE -ne 0) { throw 'Worker preflight failed. No visit started.' }
$check = ($preflight -join "`n") | ConvertFrom-Json
if ($check.label -cne $Label -or $check.validated -isnot [bool] -or -not $check.validated -or
    $check.device_accessed -isnot [bool] -or $check.device_accessed -or
    $check.lease_requested -isnot [bool] -or $check.lease_requested) {
    throw 'Frozen plan or preflight isolation mismatch. No visit started.'
}
if (-not (Confirm-ShortNotice $ReadinessNotice 35 $null $null)) { throw 'Readiness was not confirmed. No visit started.' }
$beforeClock = $clock.ElapsedMilliseconds
$clockLine = & ssh @sshOptions 'date +%s%3N'
$afterClock = $clock.ElapsedMilliseconds
if ($LASTEXITCODE -ne 0 -or "$clockLine" -notmatch '^\d{13}$' -or $afterClock - $beforeClock -ge 2000) {
    throw 'The worker clock could not be bounded. No visit started.'
}
$workerTime = [long]$clockLine
$process = [Diagnostics.Process]::new()
$process.StartInfo.FileName = (Get-Command ssh -CommandType Application).Source
$process.StartInfo.Arguments = "-T -o BatchMode=yes -o ConnectTimeout=6 -o ServerAliveInterval=5 -o ServerAliveCountMax=2 test-env `"$base --lease-tool $LeaseTool --owner $Owner --operator-ready`""
$process.StartInfo.UseShellExecute = $false
$process.StartInfo.CreateNoWindow = $true
$process.StartInfo.RedirectStandardInput = $true
$process.StartInfo.RedirectStandardOutput = $true
if ($process.StartInfo.PSObject.Properties['StandardInputEncoding']) {
    $process.StartInfo.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
}
$expected = @('admit','reference','stage')
$position = 0
$spent = $false
$started = $false
try {
    $started = $process.Start()
    if (-not $started) { throw 'Unable to start worker transport.' }
    $writer = $process.StandardInput
    $writer.NewLine = "`n"
    $secret = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($DevicePassword)
    try { $writer.WriteLine([Runtime.InteropServices.Marshal]::PtrToStringBSTR($secret)); $writer.Flush() }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($secret) }
    $pending = $process.StandardOutput.ReadLineAsync()
    while ($null -ne ($line = $pending.GetAwaiter().GetResult())) {
        $pending = $process.StandardOutput.ReadLineAsync()
        if ($line -cmatch '^(admit|reference|stage) ') {
            if ($spent -or $position -ge $expected.Count -or $line.TrimEnd(' ') -cne ($expected[$position] + ' none 0')) {
                $spent = $true
                $writer.Close()
            } else { $position++ }
        }
        if ($line -notmatch '^STARTUP_') { Write-Output $line; continue }
        if ($spent -or $position -ne $expected.Count -or $line -cnotmatch '^STARTUP_CUE ([0-9a-f]{16}) ([0-9a-f]{32}) ([0-9]{13})$') {
            $spent = $true
            $writer.Close()
            Write-Output 'Suppressed an invalid or out-of-order startup instruction.'
            continue
        }
        $spent = $true
        $cueLabel = $Matches[1]; $challenge = $Matches[2]; $issued = [long]$Matches[3]
        $ageMax = $workerTime + $clock.ElapsedMilliseconds - $beforeClock - $issued
        $confirmed = $false
        if ($cueLabel -ceq $Label -and $ageMax -ge 0 -and $ageMax -le 2000 -and -not $pending.IsCompleted) {
            $confirmed = Confirm-ShortNotice $TurnNotice 18 $process $pending
        }
        $ageMax = $workerTime + $clock.ElapsedMilliseconds - $beforeClock - $issued
        $accepted = $confirmed -and $ageMax -ge 5000 -and $ageMax -lt 23000 -and -not $pending.IsCompleted -and -not $process.HasExited
        $answer = if ($accepted) { 'STARTUP_ACK' } else { 'STARTUP_CANCEL' }
        try { $writer.WriteLine("$answer $Label $challenge"); $writer.Flush() } finally { $writer.Close() }
        if ($accepted) { Write-Output 'Startup instruction acknowledged for this attempt.' }
        else { Write-Output 'No valid startup confirmation; keep the Kindle still.' }
    }
    $process.WaitForExit()
    $exitCode = $process.ExitCode
} finally {
    if ($started) {
        try { $process.StandardInput.Close() } catch [System.IO.IOException] {}
        if (-not $process.HasExited) { $process.WaitForExit() }
    }
    $process.Dispose()
}
exit $exitCode
