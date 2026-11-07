printf '%s' "$$" > "$1"
sh "$3" "$2" &
while [ ! -f "$2" ]; do sleep 0.025; done
if [ "$4" = exit ]; then exit 0; fi
wait
