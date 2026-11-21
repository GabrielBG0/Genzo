Genzo native-surface kit (macOS, Apple silicon)
================================================

Thanks for lending your Mac. Genzo is a photo editor in development. This
tests one idea: drawing the photo straight on the GPU, under a transparent
web page that holds the buttons, so photos stay sharp while a slider moves.
It checks that the idea works on a Mac, how fast it is, and what the screen
does with the photo's colors. It installs nothing and changes nothing on your
system; everything it writes stays in this folder.

1. Plug the laptop in and turn off Low Power Mode (System Settings > Battery).
2. For the color part, please turn these off, then leave the brightness where
   it is:
     - True Tone   (System Settings > Displays)
     - Night Shift (System Settings > Displays > Night Shift)
   and use the display's normal color preset (Displays > Preset, usually
   "Apple XDR Display (P3-1600 nits)" or "Color LCD"). Do not change the preset
   during the run.
3. Open Terminal, type "cd " (with a space), drag this folder onto the
   Terminal window, press Enter, then run:

       ./run.sh

   The test build is not signed by Apple, so the script removes macOS's
   "downloaded from the internet" flag from this folder (only this folder) to
   let it start. If macOS still says it cannot verify "tauri-surface-probe",
   open System Settings > Privacy & Security, click "Open Anyway" and run the
   script again.
4. The first screenshot makes macOS ask whether Terminal may record the
   screen. Allow it (System Settings > Privacy & Security > Screen & System
   Audio Recording), then run ./run.sh again if it asked while a test window
   was open. Without the permission the timings still work; run
   "./run.sh --no-screenshots" to skip the color part.
5. It takes about 6 to 8 minutes. A test window opens and closes about 14
   times by itself, each for a few seconds. Do not touch the mouse or
   keyboard, do not cover the window, and stay at the computer: if the screen
   dims or locks, the run is lost. Near the end one window stays open for 20
   seconds with a panel that moves; you may drag its corner then, or leave it.
6. Send back the file it names at the end, genzo-surface-results-macos-*.tar.gz.
   Its name carries the code version (a short commit hash, also in the VERSION
   file here), so the results can be matched to the exact build.

If a test window shows only gray or white, or nothing opens, still send the
file: that is a result too.

What is collected: macOS version, CPU model, RAM size, GPU and display model
and resolution, whether Low Power Mode is on, the timings, screenshots of the
test window only, and the test program's own text output. No user name, host
name, other windows, files or network information. (The screenshots cover only
the test window's area (no screenshot is taken if its position is unknown),
but anything that lies over it at that moment would be in it: please keep
other windows away from it.)
