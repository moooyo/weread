#!/bin/sh
set -eu
export LC_ALL=C
document=/mnt/us/documents/weread.sh
catalog=/var/local/cc.db

kind() {
    if [ -L "$1" ]; then printf other
    elif [ -f "$1" ]; then printf regular
    elif [ -e "$1" ]; then printf other
    else printf absent; fi
}
for directory in /mnt/us/documents /mnt/us/weread /mnt/us/weread/app; do
    [ ! -L "$directory" ] || { printf 'refused linked_parent\n'; exit 1; }
done
document_kind=$(kind "$document")
printf 'document=%s\n' "$document_kind"
match=not_compared
if [ "$document_kind" = regular ]; then
    identity=$(stat -c '%d:%i:%s:%y' "$document")
    if cmp -s "$document" -; then match=equal
    else status=$?; [ "$status" -eq 1 ] || { printf 'refused document_read\n'; exit 1; }; match=different; fi
    [ "$(stat -c '%d:%i:%s:%y' "$document")" = "$identity" ] || { printf 'refused document_changed\n'; exit 1; }
fi
printf 'reference=%s\n' "$match"
printf 'supervisor=%s\n' "$(kind /mnt/us/weread/launch.sh)"
printf 'application=%s\n' "$(kind /mnt/us/weread/app/weread)"

quiet_database() {
    [ "$(kind "$catalog")" = regular ] || return 1
    for sidecar in "$catalog-wal" "$catalog-journal"; do
        [ ! -L "$sidecar" ] && [ ! -s "$sidecar" ] || return 1
    done
}
rows=not_sampled
if command -v sqlite3 >/dev/null && quiet_database &&
   sqlite3 -version | awk '{split($1,v,"."); exit !(v[1]+0>3 || (v[1]+0==3 && v[2]+0>=22))}'; then
    identity=$(stat -c '%d:%i:%s:%y' "$catalog")
    if count=$(sqlite3 -batch -noheader -readonly "file:$catalog?mode=ro&immutable=1" \
        "SELECT count(*) FROM Entries WHERE p_location COLLATE BINARY = '/mnt/us/documents/weread.sh';" 2>/dev/null) &&
       quiet_database && [ "$(stat -c '%d:%i:%s:%y' "$catalog")" = "$identity" ]; then
        case "$count" in *[!0-9]*|'') ;; *) rows=$count ;; esac
    fi
fi
printf 'selected_rows=%s\n' "$rows"
