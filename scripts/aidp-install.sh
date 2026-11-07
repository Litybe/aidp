#!/usr/bin/env bash
# aidp-install.sh - sourceable helpers for installing the aidp binary and
# migrating the shell rc PATH block. Defines functions only; sourcing it has
# no side effect. Compatible with bash 3.2.

AIDP_PATH_MARKER='# aidp v7 PATH'

aidp_install_binary() {
    local source="$1"
    local install_dir="$2"
    local target staging
    mkdir -p "$install_dir" || return 1
    target="$install_dir/$(basename "$source")"
    staging="$target.new"
    rm -f "$staging"
    if ! cp "$source" "$staging" || ! chmod +x "$staging" || ! mv -f "$staging" "$target"; then
        rm -f "$staging"
        printf 'aidp is in use at %s: close every session running aidp, then re-run /pipeline-setup-v7\n' "$target" >&2
        return 1
    fi
    printf '%s\n' "$target"
}

aidp_path_line() {
    local install_dir="$1"
    local shell_name="$2"
    if [ "$shell_name" = "fish" ]; then
        printf 'set -gx PATH "%s" $PATH\n' "$install_dir"
    else
        printf 'export PATH="%s:$PATH"\n' "$install_dir"
    fi
}

aidp_migrate_rc() {
    local rc_file="$1"
    local install_dir="$2"
    local shell_name="$3"
    local desired_line marker_count current_line had_marker=0 rc_dir tmp_file created=0

    desired_line="$(aidp_path_line "$install_dir" "$shell_name")"

    if [ -f "$rc_file" ]; then
        marker_count="$(grep -c -x -F -e "$AIDP_PATH_MARKER" "$rc_file" || true)"
        if [ "$marker_count" = "1" ]; then
            current_line="$(awk -v marker="$AIDP_PATH_MARKER" '$0 == marker { getline next_line; print next_line; exit }' "$rc_file")"
            if [ "$current_line" = "$desired_line" ]; then
                printf 'unchanged\n'
                return 0
            fi
        fi
        [ "$marker_count" != "0" ] && had_marker=1
    else
        created=1
    fi

    rc_dir="$(dirname "$rc_file")"
    mkdir -p "$rc_dir" || return 1
    tmp_file="$(mktemp "$rc_dir/.aidp-rc.XXXXXX")" || return 1

    if [ -f "$rc_file" ]; then
        awk -v marker="$AIDP_PATH_MARKER" '
            skip_next { skip_next = 0; next }
            $0 == marker { held = 0; skip_next = 1; next }
            held { print ""; held = 0 }
            $0 == "" { held = 1; next }
            { print }
            END { if (held) print "" }
        ' "$rc_file" > "$tmp_file" || { rm -f "$tmp_file"; return 1; }
    fi

    {
        printf '\n%s\n%s\n' "$AIDP_PATH_MARKER" "$desired_line"
    } >> "$tmp_file" || { rm -f "$tmp_file"; return 1; }

    [ "$created" = "1" ] && chmod 644 "$tmp_file"
    mv -f "$tmp_file" "$rc_file" || { rm -f "$tmp_file"; return 1; }

    if [ "$had_marker" = "1" ]; then
        printf 'migrated\n'
    else
        printf 'added\n'
    fi
}
