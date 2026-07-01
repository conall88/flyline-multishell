Output demo_cursor_style.gif
Output demo_cursor_style.svg

Source demo_settings.tape
Set Height 200
Set Width 1200
Source demo_setup.tape

Hide
Sleep 500ms
Type@40ms "flyline set-cursor --backend flyline --style '#33ccff' --effect fade --effect-easing in-out-sine --interpolate-easing out-elastic --interpolate 1.5 && clear"
Enter
Sleep 500ms
Show

Type "echo 'elastic cursor!'"
Sleep 1000ms
Home
Sleep 700ms
End
Sleep 1000ms
Home
Sleep 1000ms
End
Sleep 1000ms
