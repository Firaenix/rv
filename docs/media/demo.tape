# The README's hero demo: open a branch review, walk the diff, write a
# comment, glance at the keymap, leave. Regenerate with:
#   nix shell nixpkgs#vhs -c vhs docs/media/demo.tape
Output docs/media/demo.gif
Set FontSize 15
Set Width 1200
Set Height 640
Set Padding 12
Set TypingSpeed 40ms
Set Shell "bash"

Hide
Type "export PATH=/Users/firaenix/Projects/rv/target/release:$PATH && cd /tmp/rvdemo && clear"
Enter
Sleep 500ms
Show

Type "rv --from main retry-backoff"
Sleep 500ms
Enter
Sleep 2.5s
Screenshot docs/media/shot-diff.png
Down@200ms 6
Sleep 1s
Type "g"
Sleep 400ms
Down 1
Sleep 800ms
Type "]"
Sleep 1.5s
Down@150ms 8
Sleep 800ms
Type "c"
Sleep 600ms
Type "does base_delay * 2^attempt overflow for attempts > 31?"
Sleep 600ms
Enter
Sleep 1.2s
Screenshot docs/media/shot-comment.png
Type "m"
Sleep 400ms
Type "o"
Sleep 1.2s
Screenshot docs/media/shot-comments.png
Type "?"
Sleep 1s
Type "?"
Sleep 1.6s
Screenshot docs/media/shot-keys.png
Type "q"
Sleep 600ms
Type "q"
Sleep 800ms
