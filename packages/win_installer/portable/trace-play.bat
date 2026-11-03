@echo off
rem Starts Neo Aegisub with playback tracing enabled so a stutter can be
rem diagnosed without a debugger on the machine that shows it.
rem Play the clip that stutters for about ten seconds, close the player, then
rem send both log files to the developer.
setlocal
set "AEGISUB_GPUI_TRACE=%TEMP%\aegisub-gpui-frame.log"
set "AEGISUB_GPUI_HOST_TRACE=%TEMP%\aegisub-host-frame.log"
echo frame log: %AEGISUB_GPUI_TRACE%
echo host log : %AEGISUB_GPUI_HOST_TRACE%
start "" "%~dp0aegisub.exe" %*
endlocal
