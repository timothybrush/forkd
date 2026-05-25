# vhs script for the wp-bench demo, recorded as both gif and mp4 so
# README can embed the gif and Twitter/dev.to can use the mp4.
#
# Usage:  vhs docs/assets/wp-bench-demo.tape
# Deps:   vhs (charmbracelet/vhs >= 0.10), ttyd, ffmpeg
# Output: docs/assets/wp-bench-demo.gif
#         docs/assets/wp-bench-demo.mp4
#
# The run targets a 1 GiB region so the 3.18 ms WP-arm latency is the
# headline number on the v0.4 path (matches the Phase 1 PoC results).

Output docs/assets/wp-bench-demo.gif
Output docs/assets/wp-bench-demo.mp4

Set Shell "bash"
Set FontSize 18
Set Width 1280
Set Height 720
Set Padding 30
Set Theme "Catppuccin Mocha"
Set TypingSpeed 50ms
Set PlaybackSpeed 1.0

# Cold start so the viewer can see the prompt clearly.
Sleep 600ms
Type "# v0.4 prototype — measure BRANCH pause window via UFFDIO_WRITEPROTECT"
Sleep 400ms
Enter
Sleep 800ms

# The headline command. region=1024 MiB gives the 3 ms/GiB result.
Type "sudo forkd wp-bench --region-mib 1024"
Sleep 600ms
Enter

# wp-bench runs for ~7s on the dev box. Let it finish, then linger so
# viewer has time to absorb the final numbers (arm: 3.18ms, total: 7s,
# snapshot consistent).
Sleep 8s

# Annotate.
Type "# 3.18 ms WP-arm latency — vs ~150 ms in v0.3.4"
Sleep 400ms
Enter
Sleep 600ms
Type "# this is the new BRANCH 'pause window' analog"
Sleep 400ms
Enter
Sleep 3s
