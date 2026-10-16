Genzo benchmark kit (Linux)
===========================

Thanks for lending your machine. This measures how fast Genzo, a photo editor
in development, gets a slider change onto the screen. It installs nothing and
changes nothing on your system; everything it writes stays in this folder.

1. Plug the laptop in (battery mode slows the GPU) and close heavy apps.
2. Open a terminal in this folder and run:

       ./run-bench.sh

3. It takes about 10 to 15 minutes:
   - a few minutes of GPU benchmark in the terminal, nothing to do;
   - then a Genzo window opens 2 or 3 times and moves its own sliders for
     about 2 minutes each. Do not touch the mouse or keyboard, and do not
     minimize or cover the window. After each run it asks whether the photo
     visibly changed: answer y or n.
4. Send back the file it names at the end, genzo-bench-results-*.tar.gz.

If it stops with "libraries are missing", run the install line it prints for
your distribution (WebKitGTK 4.1) and start it again.

What is collected: OS and kernel version, CPU model, RAM size, GPU models,
driver and WebKitGTK versions, screen resolution, and the timings. No user
name, host name, files or network information.
