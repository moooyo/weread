[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^/root/weread-stock-chain-ab-[0-9]{8}\.[A-Za-z0-9]{8}$')][string]$Worker,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{16}$')][string]$FirstLabel,
    [Parameter(Mandatory)][ValidateRange(0,13)][int]$StartSlot,
    [Parameter(Mandatory)][ValidateRange(0,13)][int]$EndSlot,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9][A-Za-z0-9_.:-]{0,95}$')][string]$Owner,
    [Parameter(Mandatory)][Security.SecureString]$DevicePassword
)

$ErrorActionPreference = 'Stop'
if ($StartSlot -gt $EndSlot -or $DevicePassword.Length -eq 0) { throw 'A bounded range and private credential are required.' }
$command = "python3 -B $Worker/src/tools/startup-observer/run_stock_ab_range.py --work $Worker --first-label $FirstLabel run --start $StartSlot --end $EndSlot --owner $Owner"
$process = [Diagnostics.Process]::new()
$process.StartInfo.FileName = (Get-Command ssh -CommandType Application).Source
$process.StartInfo.Arguments = "-T -o BatchMode=yes -o ConnectTimeout=8 -o ServerAliveInterval=5 -o ServerAliveCountMax=2 test-env `"$command`""
$process.StartInfo.UseShellExecute = $false
$process.StartInfo.CreateNoWindow = $true
$process.StartInfo.RedirectStandardInput = $true
if ($process.StartInfo.PSObject.Properties['StandardInputEncoding']) {
    $process.StartInfo.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
}
try {
    if (-not $process.Start()) { throw 'Unable to start the worker transport.' }
    try {
        $process.StandardInput.NewLine = "`n"
        $private = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($DevicePassword)
        try { $process.StandardInput.WriteLine([Runtime.InteropServices.Marshal]::PtrToStringBSTR($private)); $process.StandardInput.Flush() }
        finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($private) }
        $process.WaitForExit()
        $status = $process.ExitCode
    } finally { $process.StandardInput.Close() }
} finally { $process.Dispose() }
exit $status
