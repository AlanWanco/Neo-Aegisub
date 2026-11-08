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
    public delegate bool EnumWindowProc(IntPtr hwnd, IntPtr param);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumWindowProc callback, IntPtr param);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowProc callback, IntPtr param);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
    public struct Point { public int X, Y; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, System.Text.StringBuilder name, int count);
    public static IntPtr FindVideoWindow(IntPtr parent, int processId) {
        IntPtr result = IntPtr.Zero;
        EnumChildWindows(parent, (hwnd, _) => {
            var name = new System.Text.StringBuilder(256);
            GetClassName(hwnd, name, name.Capacity);
            if (name.ToString() != "AegisubMpvVideo") return true;
            result = hwnd;
            return false;
        }, IntPtr.Zero);
        if (result != IntPtr.Zero) return result;
        // The mpv window is an owned popup of no child window, so it only shows
        // up among the frontend process' top-level windows.
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != (uint)processId) return true;
            var name = new System.Text.StringBuilder(256);
            GetClassName(hwnd, name, name.Capacity);
            if (name.ToString() != "AegisubMpvVideo") return true;
            result = hwnd;
            return false;
        }, IntPtr.Zero);
        return result;
    }
    public static long HitTest(IntPtr hwnd) {
        Rect rect;
        GetWindowRect(hwnd, out rect);
        int x = (rect.Left + rect.Right) / 2;
        int y = (rect.Top + rect.Bottom) / 2;
        return SendMessage(hwnd, 0x0084, IntPtr.Zero, (IntPtr)(((y & 0xFFFF) << 16) | (x & 0xFFFF))).ToInt64();
    }
    public static bool HasVisibleRenderer(IntPtr parent) {
        bool found = false;
        EnumChildWindows(parent, (hwnd, _) => {
            found = IsWindowVisible(hwnd);
            return !found;
        }, IntPtr.Zero);
        return found;
    }
    public struct Rect { public int Left, Top, Right, Bottom; }
}
'@
[void][GpuiDesktop]::SetProcessDPIAware()
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$runtime = Join-Path $OutputDirectory 'runtime'
Copy-Item -LiteralPath $PackageDirectory -Destination $runtime -Recurse
# The package ships no settings file; the app creates one on first start. Seed
# only the keys this smoke run depends on, so no first-start prompt or update check appears.
$configPath = Join-Path $runtime 'config.json'
$config = @{
    App = @{ 'First Start' = $false; Language = 'en_US'; Theme = 2; Auto = @{ 'Check For Updates' = $false } }
    Video = @{ 'PlayRes Mismatch' = 0; 'LayoutRes Mismatch' = 0; 'YCbCr Matrix Mismatch' = 0 }
}
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
    # Exercise the shipped libmpv DLL through the actual visible workspace.
    # Y4M avoids a separate encoder dependency on the Windows runner.
    $videoPath = Join-Path $runtime 'native-video-smoke.y4m'
    @'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
w, h = 320, 180
with path.open('wb') as out:
    out.write(f'YUV4MPEG2 W{w} H{h} F60:1 Ip A1:1 C420jpeg\n'.encode())
    for frame in range(1200):
        y = bytes(40 + ((x + frame * 3) % 180) for x in range(w)) * h
        out.write(b'FRAME\n' + y + bytes([96]) * (w*h//4) + bytes([192]) * (w*h//4))
'@ | python - $videoPath
    if ($LASTEXITCODE -ne 0) { throw 'Could not generate native video fixture' }
    $null = Invoke-Bridge @{ op = 'open'; kind = 'video'; path = $videoPath }
    Press-Control 'libmpv'
    $null = Invoke-Bridge @{ op = 'command'; name = 'video/play' }
    $videoWindow = [IntPtr]::Zero
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        $videoWindow = [GpuiDesktop]::FindVideoWindow($script:windowHandle, $frontend.Id)
        if ($videoWindow -ne [IntPtr]::Zero -and [GpuiDesktop]::IsWindowVisible($videoWindow) -and
            [GpuiDesktop]::HasVisibleRenderer($videoWindow)) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($videoWindow -eq [IntPtr]::Zero -or -not [GpuiDesktop]::IsWindowVisible($videoWindow) -or
        -not [GpuiDesktop]::HasVisibleRenderer($videoWindow)) {
        throw 'Native libmpv preview did not become visible'
    }
    $before = Invoke-Bridge @{ op = 'playback' }
    Start-Sleep -Milliseconds 200
    $after = Invoke-Bridge @{ op = 'playback' }
    if ($after.video_time -le $before.video_time) { throw 'Video clock did not advance during native playback' }
    $videoBounds = [GpuiDesktop+Rect]::new()
    $mainBounds = [GpuiDesktop+Rect]::new()
    [void][GpuiDesktop]::GetWindowRect($videoWindow, [ref]$videoBounds)
    [void][GpuiDesktop]::GetWindowRect($script:windowHandle, [ref]$mainBounds)
    if ($videoBounds.Right -le $videoBounds.Left -or $videoBounds.Bottom -le $videoBounds.Top -or
        $videoBounds.Left -lt $mainBounds.Left -or $videoBounds.Top -lt $mainBounds.Top -or
        $videoBounds.Right -gt $mainBounds.Right -or $videoBounds.Bottom -gt $mainBounds.Bottom) {
        throw 'Native video window lies outside the workspace'
    }
    $videoCenter = [GpuiDesktop+Point]::new()
    $videoCenter.X = [int](($videoBounds.Left + $videoBounds.Right) / 2)
    $videoCenter.Y = [int](($videoBounds.Top + $videoBounds.Bottom) / 2)
    if ([GpuiDesktop]::WindowFromPoint($videoCenter) -ne $script:windowHandle) {
        throw 'Native video steals mouse input from the GPUI workspace'
    }
    # A disabled or layered child window would swallow clicks even when hit
    # testing skips it, so ask the window itself what it answers.
    if ([GpuiDesktop]::HitTest($videoWindow) -ne -1) {
        throw 'Native video does not report HTTRANSPARENT to hit testing'
    }
    # Two frames a fraction of a second apart must differ. mpv keeps decoding
    # while DWM stops compositing its presents, so the clock check above cannot
    # see a frozen picture; only the pixels can.
    function Save-VideoFrame {
        $frameBounds = [GpuiDesktop+Rect]::new()
        [void][GpuiDesktop]::GetWindowRect($videoWindow, [ref]$frameBounds)
        $image = [Drawing.Bitmap]::new($frameBounds.Right - $frameBounds.Left, $frameBounds.Bottom - $frameBounds.Top)
        $graphics = [Drawing.Graphics]::FromImage($image)
        try { $graphics.CopyFromScreen($frameBounds.Left, $frameBounds.Top, 0, 0, $image.Size) }
        finally { $graphics.Dispose() }
        return $image
    }
    $firstFrame = Save-VideoFrame
    Start-Sleep -Milliseconds 300
    $secondFrame = Save-VideoFrame
    $changed = 0
    $shades = @{}
    try {
        for ($y = 0; $y -lt $firstFrame.Height; $y += 6) {
            for ($x = 0; $x -lt $firstFrame.Width; $x += 6) {
                $earlier = $firstFrame.GetPixel($x, $y).ToArgb()
                $later = $secondFrame.GetPixel($x, $y).ToArgb()
                if ($earlier -ne $later) { $changed++ }
                $shades[$later] = $true
            }
        }
    } finally { $firstFrame.Dispose(); $secondFrame.Dispose() }
    if ($shades.Count -lt 3) { throw 'Native video preview is a flat surface, not the picture' }
    if ($changed -lt 18) { throw 'Native video preview froze: two frames are identical' }
    Save-Screenshot 'native-video-playing'
    $originalWidth = $mainBounds.Right - $mainBounds.Left
    $originalHeight = $mainBounds.Bottom - $mainBounds.Top
    # Growing the window hands the extra width to the editor column, so the
    # video area only moves when the window shrinks.
    $shrunkWidth = [Math]::Max(640, $originalWidth - 240)
    if ($shrunkWidth -lt $originalWidth) {
        [void][GpuiDesktop]::SetWindowPos($script:windowHandle, [IntPtr]::Zero, 0, 0, $shrunkWidth, $originalHeight, 0x0006)
        Start-Sleep -Milliseconds 500
        $resizedVideo = [GpuiDesktop+Rect]::new()
        $resizedMain = [GpuiDesktop+Rect]::new()
        [void][GpuiDesktop]::GetWindowRect($videoWindow, [ref]$resizedVideo)
        [void][GpuiDesktop]::GetWindowRect($script:windowHandle, [ref]$resizedMain)
        if ($resizedVideo.Right - $resizedVideo.Left -eq $videoBounds.Right - $videoBounds.Left -and
            $resizedVideo.Bottom - $resizedVideo.Top -eq $videoBounds.Bottom - $videoBounds.Top -and
            $resizedVideo.Left -eq $videoBounds.Left -and $resizedVideo.Top -eq $videoBounds.Top) {
            throw 'Native video window did not follow workspace resize'
        }
        if ($resizedVideo.Left -lt $resizedMain.Left -or $resizedVideo.Top -lt $resizedMain.Top -or
            $resizedVideo.Right -gt $resizedMain.Right -or $resizedVideo.Bottom -gt $resizedMain.Bottom) {
            throw 'Native video window left the resized workspace'
        }
        Save-Screenshot 'native-video-resized'
        [void][GpuiDesktop]::SetWindowPos($script:windowHandle, [IntPtr]::Zero, 0, 0, $originalWidth, $originalHeight, 0x0006)
        Start-Sleep -Milliseconds 300
    }
    $null = Invoke-Bridge @{ op = 'command'; name = 'video/stop' }
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        if (-not [GpuiDesktop]::IsWindowVisible($videoWindow)) { break }
        Start-Sleep -Milliseconds 100
    }
    if ([GpuiDesktop]::IsWindowVisible($videoWindow)) { throw 'Pause left native video above the editing preview' }
    Save-Screenshot 'native-video-paused'
    Press-Control 'libmpv'
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
    # The two cards share one row: the form and the history list each keep their
    # own scroll area beside each other instead of stacking into one tall column.
    $amountRect = (Find-Control '平移时长' -Kind Edit).Current.BoundingRectangle
    if ((Find-Control '平移时长' -Kind Edit).Current.IsOffscreen) { throw 'Compact layout hides the amount field' }
    $clearRect = (Find-Control '清空…').Current.BoundingRectangle
    if ($clearRect.Left -le $amountRect.Right) { throw 'History card is not beside the parameter form' }
    Save-Screenshot 'timing-two-columns'
    # Below the wrap threshold the history drops under the form rather than
    # squeezing both columns past the width their segmented rows need.
    [void][GpuiDesktop]::SetWindowPos($script:windowHandle, [IntPtr]::Zero, $screen.X, $screen.Y, 560, 600, 0x0040)
    Start-Sleep -Milliseconds 350
    $wrappedAmount = (Find-Control '平移时长' -Kind Edit).Current.BoundingRectangle
    $wrappedClear = (Find-Control '清空…').Current.BoundingRectangle
    if ($wrappedClear.Top -le $wrappedAmount.Top) { throw 'Narrow window did not stack the history under the form' }
    Save-Screenshot 'timing-wrapped'
    Assert-Clean $base
    @{ actual_windows_desktop = $true; native_video_frames_differ = $true; native_video_click_through = $true;
       visible_amount_field = $true; exact_form_shift_ms = 1250;
       undo_restores_clean_document = $true; independent_history_scroll = $true; two_column_cards = $true;
       narrow_window_stacks_cards = $true } |
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
