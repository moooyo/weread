[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Title,
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Message,
    [ValidateRange(1, 600)][int]$TimeoutSeconds = 30,
    [switch]$Confirm
)

$ErrorActionPreference = 'Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
    -not [Environment]::UserInteractive -or
    [System.Diagnostics.Process]::GetCurrentProcess().SessionId -eq 0) {
    [Console]::Error.WriteLine('An interactive Windows desktop is required.')
    exit 3
}

$shell = $null
try {
    $shell = New-Object -ComObject WScript.Shell
    $buttons = if ($Confirm) { 1 } else { 0 }
    $informationIcon = 0x40
    $setForeground = 0x10000
    $topmost = 0x40000
    $choice = $shell.Popup($Message, $TimeoutSeconds, $Title, ($buttons -bor $informationIcon -bor $setForeground -bor $topmost))
} catch {
    [Console]::Error.WriteLine('Unable to show the operator notice.')
    exit 3
} finally {
    if ($null -ne $shell) {
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell)
    }
}

switch ($choice) {
    1 { Write-Output 'ack'; exit 0 }
    2 { Write-Output 'canceled'; exit 1 }
    -1 { Write-Output 'timed_out'; exit 2 }
    default { [Console]::Error.WriteLine('Unexpected Windows dialog result.'); exit 3 }
}
