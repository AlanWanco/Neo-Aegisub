#!/usr/bin/env pwsh
# Exercise the real portable application on a Windows runner and retain screenshots.
param (
    [Parameter(Mandatory)][string]$PackageDirectory,
    [Parameter(Mandatory)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing, System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class GpuiDesktop {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint x, uint y, int data, UIntPtr extra);
    public struct Rect { public int Left, Top, Right, Bottom; }
}
'@
[void][GpuiDesktop]::SetProcessDPIAware()
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$runtime = Join-Path $OutputDirectory 'runtime'
Copy-Item -LiteralPath $PackageDirectory -Destination $runtime -Recurse
$configPath = Join-Path $runtime 'config.json'
Copy-Item $configPath "$configPath.before-smoke"
$config = Get-Content $configPath -Raw | ConvertFrom-Json -AsHashtable
$config.App['First Start'] = $false
$config.App['Language'] = 'en_US'
$config.App['Theme'] = 2
$config.App['Auto'] = @{ 'Check For Updates' = $false }
$config | ConvertTo-Json -Depth 20 | Set-Content $configPath -Encoding utf8NoBOM
$history = @(0..49 | ForEach-Object {
    @{ filename = "Layout regression long subtitle filename concert 1920x1080 x265 FLAC record $_.ass";
       'is by time' = $true; 'is backward' = $false; amount = '0:00:01.10'; mode = 0; fields = 0;
       selection = @(@{ start = 1; end = 1 }) }
})
$history | ConvertTo-Json -Depth 20 | Set-Content (Join-Path $runtime 'shift_history.json') -Encoding utf8NoBOM
Copy-Item (Join-Path $PSScriptRoot '../gpui-prototype/assets/demo.ass') (Join-Path $runtime 'demo.ass')
$hostProcess = $null
$frontend = $null
$script:bridgeEndpoint = $null
$script:bridgeToken = $null

function Invoke-Bridge([hashtable]$Request) {
    $payload = $Request.Clone()
    $payload.token = $script:bridgeToken
    $address, $port = $script:bridgeEndpoint -split ':'
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $client.Connect($address, [int]$port)
        $client.ReceiveTimeout = 8000
        $stream = $client.GetStream()
        $bytes = [Text.Encoding]::UTF8.GetBytes(($payload | ConvertTo-Json -Compress -Depth 30) + "`n")
        $stream.Write($bytes, 0, $bytes.Length)
        $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8)
        $result = $reader.ReadToEnd() | ConvertFrom-Json -AsHashtable
        if (-not $result.ok) { throw "Bridge operation failed: $($Request.op)" }
        return $result
    } finally { $client.Dispose() }
}

function Find-Control([string]$Name, [switch]$Last, [string]$Kind = 'Button') {
    $nameCondition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $Name)
    $controlType = if ($Kind -eq 'Edit') { [Windows.Automation.ControlType]::Edit } else { [Windows.Automation.ControlType]::Button }
    $typeCondition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, $controlType)
    $condition = [Windows.Automation.AndCondition]::new($nameCondition, $typeCondition)
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        $root = [Windows.Automation.AutomationElement]::FromHandle($script:windowHandle)
        $controls = @($root.FindAll([Windows.Automation.TreeScope]::Descendants, $condition) | Where-Object { -not $_.Current.IsOffscreen })
        if ($controls.Count -gt 0) {
            if ($Last) { return $controls[$controls.Count - 1] }
            return $controls[0]
        }
        Start-Sleep -Milliseconds 200
    }
    throw "Visible control not found: $Name"
}

function Press-Control([string]$Name, [switch]$Last) {
    $element = Find-Control $Name -Last:$Last
    $pattern = $element.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)
    $pattern.Invoke()
    Start-Sleep -Milliseconds 250
}

function Switch-ToToolWindow([string] $Title) {
    # Tools open in their own window now, so the sidebar click above does not
    # change what the workspace window shows.
    $root = [Windows.Automation.AutomationElement]::RootElement
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $Title)
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        $window = $root.FindFirst([Windows.Automation.TreeScope]::Children, $condition)
        if ($window) {
            $script:windowHandle = [IntPtr]$window.Current.NativeWindowHandle
            [void][GpuiDesktop]::SetForegroundWindow($script:windowHandle)
            Start-Sleep -Milliseconds 350
            return
        }
        Start-Sleep -Milliseconds 200
    }
    throw "Tool window not found: $Title"
}

function Switch-ToMainWindow {
    $script:windowHandle = $script:mainWindowHandle
    [void][GpuiDesktop]::SetForegroundWindow($script:windowHandle)
    Start-Sleep -Milliseconds 350
}

function Set-Amount([string]$Value) {
    $element = Find-Control '平移时长' -Kind Edit
    $pattern = $element.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
    $pattern.SetValue($Value)
    Start-Sleep -Milliseconds 200
}

function Save-Screenshot([string]$Name) {
    Start-Sleep -Milliseconds 350
    $bounds = [GpuiDesktop+Rect]::new()
    [void][GpuiDesktop]::GetWindowRect($script:windowHandle, [ref]$bounds)
    $image = [Drawing.Bitmap]::new($bounds.Right - $bounds.Left, $bounds.Bottom - $bounds.Top)
    $graphics = [Drawing.Graphics]::FromImage($image)
    try {
        $graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $image.Size)
        $image.Save((Join-Path $OutputDirectory "$Name.png"), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose(); $image.Dispose() }
}

function Assert-Clean($Expected) {
    $state = Invoke-Bridge @{ op = 'snapshot' }
    if ($state.modified) { throw 'Undo left the document modified' }
    for ($index = 0; $index -lt $Expected.lines.Count; $index++) {
        if ($state.lines[$index].start -ne $Expected.lines[$index].start -or $state.lines[$index].end -ne $Expected.lines[$index].end) {
            throw "Undo did not restore line $index"
        }
    }
}

try {
    $demoPath = Join-Path $runtime 'demo.ass'
    $hostProcess = Start-Process -FilePath (Join-Path $runtime 'aegisub.exe') -ArgumentList "`"$demoPath`"" -PassThru
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        $child = Get-CimInstance Win32_Process -Filter "ParentProcessId = $($hostProcess.Id)" | Where-Object Name -eq 'aegisub-gpui-prototype.exe' | Select-Object -First 1
        if ($child) {
            $command = $child.CommandLine
            if ($command -match '--bridge\s+(\S+)\s+--token\s+(\S+)') {
                $script:bridgeEndpoint = $Matches[1]
                $script:bridgeToken = $Matches[2]
                $frontend = Get-Process -Id $child.ProcessId
                if ($frontend.MainWindowHandle -ne [IntPtr]::Zero) { break }
            }
        }
        if ($hostProcess.HasExited) { throw "Host exited during startup: $($hostProcess.ExitCode)" }
        Start-Sleep -Milliseconds 200
    }
    if (-not $frontend -or $frontend.MainWindowHandle -eq [IntPtr]::Zero) { throw 'GPUI did not create a window' }
    $script:windowHandle = $frontend.MainWindowHandle
    $script:mainWindowHandle = $script:windowHandle
    $screen = [Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    [void][GpuiDesktop]::SetWindowPos($script:windowHandle, [IntPtr]::Zero, $screen.X, $screen.Y, [Math]::Min(1024, $screen.Width), [Math]::Min(720, $screen.Height), 0x0040)
    [void][GpuiDesktop]::SetForegroundWindow($script:windowHandle)
    Start-Sleep -Milliseconds 750
    Press-Control '时间平移'
    Switch-ToToolWindow 'Neo Aegisub · 时间平移'
    $input = Find-Control '平移时长' -Kind Edit
    $initialBounds = $input.Current.BoundingRectangle
    if ($input.Current.IsOffscreen -or $initialBounds.Height -lt 15) { throw 'Shift input is clipped or offscreen' }
    Save-Screenshot 'timing-initial'

    # Exercise the visible controls and verify resulting document times through the host.
    $base = Invoke-Bridge @{ op = 'snapshot' }
    Press-Control '按时间'
    Press-Control '向后延迟'
    Press-Control '全部行'
    Press-Control '开始和结束'
    Set-Amount '0:00:01.25'
    Press-Control '应用平移'
    $shifted = Invoke-Bridge @{ op = 'snapshot' }
    if (-not $shifted.modified) { throw 'Applying the form did not modify the document' }
    for ($index = 0; $index -lt $base.lines.Count; $index++) {
        if ($shifted.lines[$index].start -ne $base.lines[$index].start + 1250 -or $shifted.lines[$index].end -ne $base.lines[$index].end + 1250) {
            throw "Incorrect visible form shift on line $index"
        }
    }
    Press-Control '撤销' -Last
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        if (-not (Invoke-Bridge @{ op = 'snapshot' }).modified) { break }
        Start-Sleep -Milliseconds 100
    }
    Assert-Clean $base

    # Scroll the independent history list; its movement must not move the input.
    $clearHistory = (Find-Control '清空…').Current.BoundingRectangle
    [void][GpuiDesktop]::SetCursorPos(
        [int](($clearHistory.Left + $clearHistory.Right) / 2),
        [int]($clearHistory.Bottom + 120))
    [GpuiDesktop]::mouse_event(0x0800, 0, 0, -12000, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 500
    $afterBounds = (Find-Control '平移时长' -Kind Edit).Current.BoundingRectangle
    if ([Math]::Abs($afterBounds.Y - $initialBounds.Y) -gt 1) { throw 'History scrolling moved the parameter input' }
    Save-Screenshot 'timing-history-scrolled'

    # Each iteration ends focused on the tool window, so the sidebar click has to
    # come after switching back, not once before the loop.
    foreach ($name in @('时间后处理', '重采样', '查找替换', '样式管理')) {
        Switch-ToMainWindow
        Press-Control $name
        Switch-ToToolWindow "Neo Aegisub · $name"
        Save-Screenshot ("tool-" + $name)
    }
    Switch-ToMainWindow
    Press-Control '时间平移'
    Switch-ToToolWindow 'Neo Aegisub · 时间平移'
    [void][GpuiDesktop]::SetWindowPos($script:windowHandle, [IntPtr]::Zero, $screen.X, $screen.Y, 800, 600, 0x0040)
    Start-Sleep -Milliseconds 350
    # The compact window stacks both cards: the parameter field stays visible and
    # the history card keeps its own scroll area underneath.
    if ((Find-Control '平移时长' -Kind Edit).Current.IsOffscreen) { throw 'Compact layout hides the amount field' }
    Save-Screenshot 'timing-compact-parameters'
    if ((Find-Control '清空…').Current.IsOffscreen) { throw 'Compact layout hides the history card' }
    Save-Screenshot 'timing-compact-history'
    Assert-Clean $base
    @{ actual_windows_desktop = $true; visible_amount_field = $true; exact_form_shift_ms = 1250;
       undo_restores_clean_document = $true; independent_history_scroll = $true; compact_stacked_cards = $true } |
        ConvertTo-Json | Set-Content (Join-Path $OutputDirectory 'result.json')
    Write-Host 'Windows GPUI desktop smoke: PASS'
} finally {
    if ($frontend) {
        if (-not $frontend.HasExited) {
            try { Save-Screenshot 'final' } catch { Write-Warning "Final screenshot failed: $($_.Exception.Message)" }
        }
        Stop-Process -Id $frontend.Id -Force -ErrorAction SilentlyContinue
    }
    if ($hostProcess) { Stop-Process -Id $hostProcess.Id -Force -ErrorAction SilentlyContinue }
}
