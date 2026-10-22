playerstv {VERSION}
==================

A replay recorder for Windows: it notices when your game opens, keeps the
last 30 seconds ready, and saves them as a clip when you press a hotkey.


RUNNING IT
----------
1. Unzip this folder anywhere (e.g. your Desktop or Documents).
2. Double-click playerstv.exe.
   Windows may say "Windows protected your PC" because the app isn't signed:
   click "More info", then "Run anyway".
3. The playerstv window opens. Closing it does NOT quit: playerstv keeps
   recording from the system tray (bottom-right, near the clock; it may be
   under the ^ arrow). Left-click the tray icon to open the window again.
4. To exit completely: right-click the tray icon > Quit, or use
   "Quit playerstv" at the bottom of the window's settings.

The first start takes a few seconds longer while it unpacks its video tools.


USING IT
--------
- Start one of the supported games: Old School RuneScape (RuneLite),
  League of Legends (in matches) or Counter-Strike 2. Turn games on/off in
  the window or the tray menu. The tray dot turns green when it's ready.
- Alt+F10   save the last 30 seconds as a clip
- Alt+F9    start/stop recording the whole session (tray dot turns red)
- Clips appear in the window's library, where you can play and trim them.
  The files are in  Videos\playerstv\<game>\
- Hotkeys, quality, audio and the clip length are in the window's settings.


NEEDS
-----
- Windows 10 (version 2004 or newer) or Windows 11
- An NVIDIA graphics card (it records with the card's NVENC encoder)
- Microsoft Edge WebView2 Runtime for the window. It's already on Windows 11
  and most Windows 10 PCs; if it's missing, playerstv says so and offers
  the download.


IF SOMETHING GOES WRONG
-----------------------
Please send the newest file from this folder along with what happened:
    %LOCALAPPDATA%\playerstv\logs
(paste that into the File Explorer address bar)


REMOVING IT
-----------
1. If you turned on "Start with Windows" in settings, turn it off.
2. Right-click the tray icon > Quit, then delete this folder.
3. Optionally delete its settings and cache:
       %APPDATA%\playerstv
       %LOCALAPPDATA%\playerstv
   Your recordings in Videos\playerstv are kept unless you delete them.
