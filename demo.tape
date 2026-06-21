# vhs tape for brim's README demo. Render with: vhs demo.tape
# Produces assets/demo.gif. Requires `brim` on PATH and at least one provider
# authenticated (or it will show the not-configured state, which is fine).
# https://github.com/charmbracelet/vhs

Output assets/demo.gif

Set FontSize 18
Set Width 1100
Set Height 640
Set Padding 20
Set Theme "Catppuccin Mocha"

Type "brim status"
Sleep 500ms
Enter
Sleep 3s

Type "brim json"
Sleep 500ms
Enter
Sleep 3s

Type "brim history --days 7"
Sleep 500ms
Enter
Sleep 3s
