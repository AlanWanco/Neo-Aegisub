@echo off
rem Starts Neo Aegisub with playback tracing enabled so a stutter can be
rem diagnosed without a debugger on the machine that shows it.
rem Play the clip that stutters for about ten seconds, close the player, then
rem send the native OpenGL frame log to the developer.
setlocal
set "AEGISUB_GPUI_HOST_TRACE=%TEMP%\aegisub-host-frame.log"
echo host log : %AEGISUB_GPUI_HOST_TRACE%
start "" "%~dp0aegisub.exe" %*
endlocal
