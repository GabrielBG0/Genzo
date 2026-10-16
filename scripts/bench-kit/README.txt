Genzo benchmark kit (Linux and macOS)
=====================================

Thanks for lending your machine. This measures how fast Genzo, a photo editor
in development, gets a slider change onto the screen. It installs nothing and
changes nothing on your system; everything it writes stays in this folder.

1. Plug the laptop in (battery mode slows the GPU), turn off Low Power Mode
   on a Mac, and close heavy apps.
2. Open a terminal in this folder and run:

       ./run-bench.sh

   On a Mac: open Terminal, type "cd " (with a space), drag this folder onto
   the Terminal window, press Enter, then run the line above. The test build
   is not signed by Apple, so the script removes macOS's "downloaded from the
   internet" flag from this folder (only this folder) to let it start. If
   macOS still says it cannot verify "genzo", open System Settings > Privacy
   & Security, click "Open Anyway" and run the script again.

3. It takes about 10 to 15 minutes:
   - a few minutes of GPU benchmark in the terminal, nothing to do;
   - then a Genzo window opens (2 or 3 times on Linux, once on a Mac) and
     moves its own sliders for about a minute each. Do not touch the mouse
     or keyboard, and do not minimize or cover the window. After each run it asks whether the photo
     visibly changed: answer y or n.
4. Send back the file it names at the end, genzo-bench-results-*.tar.gz.

Optional on Linux, makes the results more useful: install glxinfo and
eglinfo first (Ubuntu and Debian: sudo apt install mesa-utils; Fedora:
sudo dnf install glx-utils egl-utils).

On Linux, if it stops with "libraries are missing", run the install line it prints for
your distribution (WebKitGTK 4.1) and start it again.

What is collected: OS and kernel version, CPU model, RAM size, GPU models,
driver and WebKit versions, screen resolution, and the timings. No user
name, host name, files or network information.
