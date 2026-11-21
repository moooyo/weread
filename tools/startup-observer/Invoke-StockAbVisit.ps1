[CmdletBinding(DefaultParameterSetName='Visit')]
param(
    [Parameter(Mandatory)][ValidatePattern('^/root/weread-[A-Za-z0-9_-]+\.[A-Za-z0-9]+$')][string]$Worker,
    [Parameter(Mandatory)][ValidatePattern('^(0[0-9]|1[0-3])-[AB]$')][string]$Visit,
    [Parameter(Mandatory,ParameterSetName='Preflight')][switch]$Preflight,
    [Parameter(Mandatory,ParameterSetName='Recover')][switch]$Recover,
    [Parameter(Mandatory,ParameterSetName='Visit')][Parameter(Mandatory,ParameterSetName='Recover')]
    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9_.:-]{0,95}$')][string]$Owner,
    [Parameter(ParameterSetName='Visit')][Parameter(ParameterSetName='Recover')]
    [ValidatePattern('^/root/weread-[A-Za-z0-9_-]+\.[A-Za-z0-9]+/device_lease\.py$')]
    [string]$LeaseTool='/root/weread-orientation-20261121.WR7ofkGL/device_lease.py',
    [Parameter(Mandatory,ParameterSetName='Visit')][Parameter(Mandatory,ParameterSetName='Recover')]
    [Security.SecureString]$DevicePassword
)

$ErrorActionPreference='Stop'
$plan="$Worker/stock-library-ab/visits/$Visit"
$command="python3 -B $Worker/src/tools/startup-observer/run_stock_ab_visit.py --work $Worker --plan $plan"
if ($Preflight) { $command+=' --preflight' }
else {
    if ($DevicePassword.Length -eq 0) { throw 'A nonempty credential is required.' }
    $command+=" --owner $Owner --lease-tool $LeaseTool"
    if ($Recover) { $command+=" --recover $plan/attempt" }
}
$process=[Diagnostics.Process]::new()
$process.StartInfo.FileName=(Get-Command ssh -CommandType Application).Source
$process.StartInfo.Arguments="-T -o BatchMode=yes -o ConnectTimeout=8 test-env `"$command`""
$process.StartInfo.UseShellExecute=$false
$process.StartInfo.CreateNoWindow=$true
$process.StartInfo.RedirectStandardInput=$true
if ($process.StartInfo.PSObject.Properties['StandardInputEncoding']) {
    $process.StartInfo.StandardInputEncoding=[Text.UTF8Encoding]::new($false)
}
try {
    if (-not $process.Start()) { throw 'Unable to start the worker transport.' }
    try {
        if (-not $Preflight) {
            $private=[Runtime.InteropServices.Marshal]::SecureStringToBSTR($DevicePassword)
            try { $process.StandardInput.WriteLine([Runtime.InteropServices.Marshal]::PtrToStringBSTR($private)) }
            finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($private) }
        }
    } finally { $process.StandardInput.Close() }
    $process.WaitForExit()
    $status=$process.ExitCode
} finally { $process.Dispose() }
exit $status
