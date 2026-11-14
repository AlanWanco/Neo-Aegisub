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
# Options live in the user directory (%APPDATA%\Aegisub on Windows), never next
# to the executable, so a settings file in the package folder is ignored. Seed
# only the keys this run depends on: the fixture video is untagged, and an open
# prompt is a host notice that covers the window, where the native surface has
# to step aside — these checks need no notice on screen at all.
$configPath = Join-Path $env:APPDATA 'Aegisub/config.json'
New-Item -ItemType Directory -Force (Split-Path -Parent $configPath) | Out-Null
$config = @{
    Video = @{
        'PlayRes Mismatch' = 0
        'LayoutRes Mismatch' = 0
        'No LayoutRes in Script' = 0
        'No YCbCr Matrix in Script' = 0
        'YCbCr Matrix Mismatch' = 0
        'Untagged Matrix Warning' = $false
        'HDR Video Warning' = $false
    }
}
$config | ConvertTo-Json -Depth 20 | Set-Content $configPath -Encoding utf8NoBOM
$history = @(0..49 | ForEach-Object {
    @{ filename = "Layout regression long subtitle filename concert 1920x1080 x265 FLAC record $_.ass";
       'is by time' = $true; 'is backward' = $false; amount = '0:00:01.10'; mode = 0; fields = 0;
       selection = @(@{ start = 1; end = 1 }) }
})
$history | ConvertTo-Json -Depth 20 | Set-Content (Join-Path $env:APPDATA 'Aegisub/shift_history.json') -Encoding utf8NoBOM
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

function Wait-Layout([scriptblock]$Sample, [hashtable]$SampleArguments = @{}, [int]$TimeoutMs = 6000) {
    # Accessibility bounds are re-published on the next frame, so a sample taken
    # right after a layout change can still describe the previous one.  Repeat
    # the whole sample until two consecutive ones agree: a check that compares
    # two controls then reads one frame instead of two, and every rectangle has
    # to have real extent so a placeholder never passes the check.  The sample
    # returns a name -> rectangle table, so callers read fields by name and a
    # single-entry sample keeps the same shape as a two-entry one.
    $previous = $null
    $lastUnusable = ''
    $lastFailure = $null
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $current = & $Sample @SampleArguments
        } catch {
            # A provider element that was rebuilt on the next frame answers an
            # old reference with an error instead of a rectangle; the retry
            # resolves a fresh one.
            $lastFailure = $_.Exception.Message
            Start-Sleep -Milliseconds 120
            continue
        }
        $lastFailure = $null
        if ($current -isnot [Collections.IDictionary]) {
            throw 'Layout sample did not return a rectangle table'
        }
        $key = ($current.Keys | Sort-Object | ForEach-Object {
            '{0}={1:N0},{2:N0},{3:N0},{4:N0}' -f $_, $current[$_].Left, $current[$_].Top, $current[$_].Width, $current[$_].Height
        }) -join ';'
        # A control can answer with no rectangle at all while its frame is being
        # replaced; count the unusable entries instead of testing the pipeline
        # for truth, because a single null result also reads as false.
        $unusable = @(foreach ($entryName in $current.Keys) {
            $entryRect = $current[$entryName]
            if (-not $entryRect -or $entryRect.Width -le 0 -or $entryRect.Height -le 0) { $entryName }
        })
        if ($previous -eq $key -and $unusable.Count -eq 0) { return $current }
        $previous = $key
        $lastUnusable = $unusable -join ','
        Start-Sleep -Milliseconds 120
    }
    if ($lastFailure) { throw "Control geometry did not settle: $lastFailure" }
    throw "Control geometry did not settle: $key (unusable: $lastUnusable)"
}

function Wait-ControlRect([string]$Name, [string]$Kind = 'Button') {
    # The lookup parameters travel as sample arguments, not as free variables:
    # a script block resolves those up the call stack, case-insensitively, so a
    # local inside Wait-Layout could otherwise shadow them.
    $sample = { param($ControlName, $ControlKind)
        @{ target = (Find-Control $ControlName -Kind $ControlKind).Current.BoundingRectangle } }
    return (Wait-Layout $sample @{ ControlName = $Name; ControlKind = $Kind }).target
}

function Get-ControlPattern([string]$Name, $Pattern, [string]$Kind = 'Button', [switch]$Last, [int]$TimeoutMs = 6000) {
    # The provider rebuilds its elements as the window redraws, so an element
    # captured a moment earlier can be detached and report no patterns at all.
    # Resolve the control again for every attempt instead of asking the same
    # element twice, and say which pattern was missing when the wait expires.
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    $detail = 'control not found'
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $element = Find-Control $Name -Kind $Kind -Last:$Last
            $resolved = $null
            if ($element.TryGetCurrentPattern($Pattern, [ref]$resolved)) { return $resolved }
            $detail = "control exposes no $($Pattern.ProgrammaticName)"
        } catch {
            $detail = $_.Exception.Message
        }
        Start-Sleep -Milliseconds 100
    }
    throw "Control $Name never accepted $($Pattern.ProgrammaticName): $detail"
}

function Wait-WindowReady([IntPtr]$Handle, [int]$TimeoutMs = 4000) {
    # A freshly activated window reports its final bounds one frame later; wait
    # for a stable pair of reads instead of assuming a fixed delay.
    $previous = $null
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ([GpuiDesktop]::IsWindowVisible($Handle)) {
            $rect = ([Windows.Automation.AutomationElement]::FromHandle($Handle)).Current.BoundingRectangle
            if ($rect.Width -gt 0 -and $rect.Height -gt 0 -and $previous -and
                [Math]::Abs($rect.Width - $previous.Width) -le 1 -and [Math]::Abs($rect.Height - $previous.Height) -le 1) {
                return
            }
            $previous = $rect
        }
        Start-Sleep -Milliseconds 120
    }
}

function Wait-WindowRect([int]$Width, [int]$Height, [int]$TimeoutMs = 4000) {
    # SetWindowPos only posts the resize: wait until the native frame reports the
    # requested size, so a threshold check can never be decided by the previous
    # size.  GetWindowRect and SetWindowPos both speak physical pixels, so this
    # comparison is unaffected by the runner's DPI.
    $bounds = [GpuiDesktop+Rect]::new()
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        [void][GpuiDesktop]::GetWindowRect($script:windowHandle, [ref]$bounds)
        $currentWidth = $bounds.Right - $bounds.Left
        $currentHeight = $bounds.Bottom - $bounds.Top
        if ([Math]::Abs($currentWidth - $Width) -le 1 -and [Math]::Abs($currentHeight - $Height) -le 1) { return }
        Start-Sleep -Milliseconds 80
    }
    throw ("Window did not reach {0}x{1}: {2}x{3}" -f $Width, $Height, $currentWidth, $currentHeight)
}

function Press-Control([string]$Name, [switch]$Last) {
    $pattern = Get-ControlPattern $Name ([Windows.Automation.InvokePattern]::Pattern) -Last:$Last
    $pattern.Invoke()
    Start-Sleep -Milliseconds 250
}

function Switch-ToToolWindow([string] $Title) {
    # Owned tool windows appear beneath the workspace in the automation tree.
    $root = [Windows.Automation.AutomationElement]::FromHandle($script:mainWindowHandle)
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $Title)
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        $window = $root.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
        if ($window) {
            $script:windowHandle = [IntPtr]$window.Current.NativeWindowHandle
            [void][GpuiDesktop]::SetForegroundWindow($script:windowHandle)
            Wait-WindowReady $script:windowHandle
            return
        }
        Start-Sleep -Milliseconds 200
    }
    throw "Tool window not found: $Title"
}

function Switch-ToMainWindow {
    $script:windowHandle = $script:mainWindowHandle
    [void][GpuiDesktop]::SetForegroundWindow($script:windowHandle)
    Wait-WindowReady $script:windowHandle
}

function Set-Amount([string]$Value) {
    $pattern = Get-ControlPattern '平移时长' ([Windows.Automation.ValuePattern]::Pattern) -Kind Edit
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
        # Opening another file rebuilds the mpv surface, so the handle is
        # resolved again per capture: a stale one reads as a zero-size window.
        $script:videoWindow = [GpuiDesktop]::FindVideoWindow($script:windowHandle, $frontend.Id)
        $frameBounds = [GpuiDesktop+Rect]::new()
        if ($script:videoWindow -ne [IntPtr]::Zero) {
            [void][GpuiDesktop]::GetWindowRect($script:videoWindow, [ref]$frameBounds)
        }
        $width = $frameBounds.Right - $frameBounds.Left
        $height = $frameBounds.Bottom - $frameBounds.Top
        if ($width -le 0 -or $height -le 0) {
            throw "Native video window has no area: ${width}x${height} handle=$($script:videoWindow)"
        }
        $image = [Drawing.Bitmap]::new($width, $height)
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
    $playingBounds = [GpuiDesktop+Rect]::new()
    [void][GpuiDesktop]::GetWindowRect($videoWindow, [ref]$playingBounds)
    $null = Invoke-Bridge @{ op = 'command'; name = 'video/stop' }
    Start-Sleep -Milliseconds 400
    # A pause keeps mpv on the picture. Handing it back to the workspace repaints
    # the same rect from a cached frame, and that swap is what moved and flashed
    # the picture, so the window has to survive the pause exactly where it was.
    if (-not [GpuiDesktop]::IsWindowVisible($videoWindow)) { throw 'Pause dropped the native picture back into the workspace' }
    $pausedBounds = [GpuiDesktop+Rect]::new()
    [void][GpuiDesktop]::GetWindowRect($videoWindow, [ref]$pausedBounds)
    if ($pausedBounds.Left -ne $playingBounds.Left -or $pausedBounds.Top -ne $playingBounds.Top -or
        $pausedBounds.Right -ne $playingBounds.Right -or $pausedBounds.Bottom -ne $playingBounds.Bottom) {
        throw 'Pause moved or resized the native video window'
    }
    Save-Screenshot 'native-video-paused'

    # Pausing keeps mpv painting, so the paused picture has to be the frame
    # playback stopped on. A flat colour names each second by hue, so a pause
    # that falls back to a cached frame shows up as the wrong colour.
    $colourPath = Join-Path $runtime 'native-video-colour.y4m'
    @'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
w, h, fps = 320, 180, 60
colours = [(235,128,128),(81,90,240),(145,54,34),(41,240,110),
           (210,16,146),(107,202,222),(170,166,16),(128,128,128)]
with path.open('wb') as out:
    out.write(f'YUV4MPEG2 W{w} H{h} F{fps}:1 Ip A1:1 C420jpeg\n'.encode())
    for frame in range(fps * 16):
        y, u, v = colours[(frame // fps) % len(colours)]
        out.write(b'FRAME\n')
        out.write(bytes([y]) * (w*h) + bytes([u]) * (w*h//4) + bytes([v]) * (w*h//4))
'@ | python - $colourPath
    if ($LASTEXITCODE -ne 0) { throw 'Could not generate the colour video fixture' }

    # One capture of the video window: its averaged colour plus the share of
    # pixels that read as green or red. The picture is letterboxed inside the
    # window and the workspace card paints its own frame over the same rect, so
    # a mean is too diluted to name the frame; the share is not.
    function Measure-VideoFrame {
        $image = Save-VideoFrame
        try {
            $sumR = 0; $sumG = 0; $sumB = 0; $count = 0; $green = 0; $red = 0
            for ($y = 0; $y -lt $image.Height; $y += 4) {
                for ($x = 0; $x -lt $image.Width; $x += 4) {
                    $pixel = $image.GetPixel($x, $y)
                    $r = $pixel.R; $g = $pixel.G; $b = $pixel.B
                    $sumR += $r; $sumG += $g; $sumB += $b; $count++
                    if ($g -ge 90 -and ($g - $r) -ge 40 -and ($g - $b) -ge 40) { $green++ }
                    if ($r -ge 90 -and ($r - $g) -ge 60 -and ($r - $b) -ge 60) { $red++ }
                }
            }
            if ($count -eq 0) { $count = 1 }
            return [pscustomobject]@{
                Mean = @([int]($sumR / $count), [int]($sumG / $count), [int]($sumB / $count))
                Green = $green / $count
                Red = $red / $count
            }
        } finally { $image.Dispose() }
    }
    function Format-Colour($Colour) { return ('{0},{1},{2}' -f $Colour[0], $Colour[1], $Colour[2]) }
    # Captures the window as fast as the screen allows, with the instant each
    # capture started and the window's visibility at that instant.
    function Watch-VideoFrames([int]$Count) {
        $frames = @()
        $watch = [Diagnostics.Stopwatch]::StartNew()
        for ($index = 0; $index -lt $Count; $index++) {
            $sample = Measure-VideoFrame
            $frames += ,@($sample.Mean, [int]$watch.ElapsedMilliseconds, [GpuiDesktop]::IsWindowVisible($videoWindow), $sample.Green, $sample.Red)
        }
        return ,$frames
    }

    $null = Invoke-Bridge @{ op = 'open'; kind = 'video'; path = $colourPath }
    $null = Invoke-Bridge @{ op = 'command'; name = 'video/play' }
    # A new file rebuilds the surface, so the window found above is gone.
    $videoWindow = [IntPtr]::Zero
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        $videoWindow = [GpuiDesktop]::FindVideoWindow($script:windowHandle, $frontend.Id)
        if ($videoWindow -ne [IntPtr]::Zero -and [GpuiDesktop]::IsWindowVisible($videoWindow)) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($videoWindow -eq [IntPtr]::Zero) { throw 'Colour video: native preview did not appear' }
    # A poll taken before the host swaps files still reports the previous
    # clip's clock, which would let the wait below pass on stale time.
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        $state = Invoke-Bridge @{ op = 'playback' }
        if ($state.video_time -lt 1000) { break }
        Start-Sleep -Milliseconds 50
    }
    $state = $null
    for ($attempt = 0; $attempt -lt 120; $attempt++) {
        $state = Invoke-Bridge @{ op = 'playback' }
        if ($state.video_time -ge 2400) { break }
        Start-Sleep -Milliseconds 60
    }
    if (-not $state -or $state.video_time -lt 2400) { throw 'Colour video did not reach its second second' }
    $pauseSample = Measure-VideoFrame
    $null = Invoke-Bridge @{ op = 'command'; name = 'video/stop' }
    Start-Sleep -Milliseconds 400
    if (-not [GpuiDesktop]::IsWindowVisible($videoWindow)) { throw 'Pause dropped the native picture back into the workspace' }
    $pausedSample = Measure-VideoFrame
    if ($pausedSample.Green -lt 0.02) {
        throw "Paused picture $(Format-Colour $pausedSample.Mean) is not the pause frame $(Format-Colour $pauseSample.Mean): green share $($pausedSample.Green)"
    }
    # Playback continues from the frame it stopped on, so the picture stays the
    # same instead of jumping back to the frame the workspace had cached.
    $null = Invoke-Bridge @{ op = 'command'; name = 'video/play' }
    $resumeFrames = Watch-VideoFrames 48
    $firstVisible = $resumeFrames | Where-Object { $_[2] } | Select-Object -First 1
    if (-not $firstVisible) { throw 'Resume never showed the native video window' }
    if ($firstVisible[1] -gt 800) { throw "Resume showed the picture only after $($firstVisible[1]) ms" }
    if ($firstVisible[3] -lt 0.02) {
        throw "Resume revealed $(Format-Colour $firstVisible[0]) instead of the paused picture: green share $($firstVisible[3])"
    }
    $null = Invoke-Bridge @{ op = 'command'; name = 'video/stop' }
    Start-Sleep -Milliseconds 300
    # A seek made while paused has to reach mpv: the front end forwards it, so
    # the picture changes before playback resumes instead of after it.
    $null = Invoke-Bridge @{ op = 'seek'; frame = 540 }
    Start-Sleep -Milliseconds 500
    $seekSample = Measure-VideoFrame
    if ($seekSample.Red -lt 0.02) {
        throw "Paused seek did not reach mpv: $(Format-Colour $seekSample.Mean) is not the frame at 540: red share $($seekSample.Red)"
    }
    $null = Invoke-Bridge @{ op = 'command'; name = 'video/play' }
    $seekFrames = Watch-VideoFrames 24
    $firstSeekVisible = $seekFrames | Where-Object { $_[2] } | Select-Object -First 1
    if (-not $firstSeekVisible) { throw 'Resume after a paused seek never showed the native video window' }
    if ($firstSeekVisible[1] -gt 800) { throw "Resume after a paused seek took $($firstSeekVisible[1]) ms" }
    if ($firstSeekVisible[4] -lt 0.02) {
        throw "Resume after a paused seek showed $(Format-Colour $firstSeekVisible[0]) instead of the seeked frame: red share $($firstSeekVisible[4])"
    }
    $null = Invoke-Bridge @{ op = 'command'; name = 'video/stop' }
    Press-Control '时间平移'
    Switch-ToToolWindow 'Neo Aegisub · 时间平移'
    $initialBounds = Wait-ControlRect '平移时长' -Kind Edit
    if ($initialBounds.Height -lt 15) { throw 'Shift input is clipped or offscreen' }
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
    $clearHistory = Wait-ControlRect '清空…'
    [void][GpuiDesktop]::SetCursorPos(
        [int](($clearHistory.Left + $clearHistory.Right) / 2),
        [int]($clearHistory.Bottom + 120))
    [GpuiDesktop]::mouse_event(0x0800, 0, 0, -12000, [UIntPtr]::Zero)
    $afterBounds = Wait-ControlRect '平移时长' -Kind Edit
    if ([Math]::Abs($afterBounds.Top - $initialBounds.Top) -gt 1) { throw 'History scrolling moved the parameter input' }
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
    [void][GpuiDesktop]::SetForegroundWindow($script:windowHandle)
    Wait-WindowRect 800 600
    # The two cards share one row: the form and the history list each keep their
    # own scroll area beside each other instead of stacking into one tall column.
    # Both rectangles come from one settled sample, so the pair describes a
    # single frame instead of two different layouts.
    $layout = Wait-Layout { @{
        amount = (Find-Control '平移时长' -Kind Edit).Current.BoundingRectangle
        clear  = (Find-Control '清空…').Current.BoundingRectangle } }
    $amountRect = $layout.amount
    $clearRect = $layout.clear
    Save-Screenshot 'timing-two-columns'
    if ($clearRect.Left -le $amountRect.Right) {
        throw ("History card is not beside the parameter form (amount={0},{1}-{2},{3} clear={4},{5}-{6},{7})" -f
            $amountRect.Left, $amountRect.Top, $amountRect.Right, $amountRect.Bottom,
            $clearRect.Left, $clearRect.Top, $clearRect.Right, $clearRect.Bottom)
    }
    # Below the wrap threshold the history drops under the form rather than
    # squeezing both columns past the width their segmented rows need.
    [void][GpuiDesktop]::SetWindowPos($script:windowHandle, [IntPtr]::Zero, $screen.X, $screen.Y, 560, 600, 0x0040)
    [void][GpuiDesktop]::SetForegroundWindow($script:windowHandle)
    Wait-WindowRect 560 600
    $layout = Wait-Layout { @{
        amount = (Find-Control '平移时长' -Kind Edit).Current.BoundingRectangle
        clear  = (Find-Control '清空…').Current.BoundingRectangle } }
    $wrappedAmount = $layout.amount
    $wrappedClear = $layout.clear
    Save-Screenshot 'timing-wrapped'
    if ($wrappedClear.Top -le $wrappedAmount.Top) {
        $wrappedBounds = [GpuiDesktop+Rect]::new()
        [void][GpuiDesktop]::GetWindowRect($script:windowHandle, [ref]$wrappedBounds)
        $wrappedRoot = ([Windows.Automation.AutomationElement]::FromHandle($script:windowHandle)).Current.BoundingRectangle
        throw ("Narrow window did not stack the history under the form (window={0}x{1} client={2}x{3} amount={4},{5} clear={6},{7})" -f
            ($wrappedBounds.Right - $wrappedBounds.Left), ($wrappedBounds.Bottom - $wrappedBounds.Top),
            ($wrappedRoot.Right - $wrappedRoot.Left), ($wrappedRoot.Bottom - $wrappedRoot.Top),
            $wrappedAmount.Left, $wrappedAmount.Top, $wrappedClear.Left, $wrappedClear.Top)
    }
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
