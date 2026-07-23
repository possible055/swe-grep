#!/bin/sh

set -eu

REPOSITORY="possible055/swe-grep"
ARCHIVE="swe-grep-linux-x86_64-musl.tar.gz"
DEFAULT_RELEASE_BASE_URL="https://github.com/${REPOSITORY}/releases/latest/download"

host=""
scope="user"
custom_root=""
release_base_url="$DEFAULT_RELEASE_BASE_URL"
force="false"
assume_yes="false"
uninstall="false"

usage() {
    cat <<'EOF'
Install swe-grep as an Agent Skill on Linux.

Usage:
  install.sh [options]

Options:
  --host agents|cursor|claude|custom
                              Skill host. Omit to choose interactively.
  --scope user|project        Install scope for a built-in host (default: user).
  --path DIRECTORY            Custom skill parent directory. Relative paths and ~/ are supported.
  --release-base-url URL      Directory containing the release archive and SHA-256 file.
  --uninstall                 Remove the selected Skill and shared runtime when it is the last one.
  --force                     Preserve managed installations as timestamped backups before replacing them.
  --yes                       Do not ask for confirmation.
  --help                      Show this help.

Built-in directories:
  agents  user: ~/.agents/skills       project: <git-root>/.agents/skills
  cursor  user: ~/.cursor/skills       project: <git-root>/.cursor/skills
  claude  user: ~/.claude/skills       project: <git-root>/.claude/skills

Compatibility alias:
  codex   Same as agents.

The launcher is always installed at ~/.local/bin/swe-grep.
EOF
}

die() {
    printf '%s\n' "Error: $*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "Required command is unavailable: $1"
}

validate_environment() {
    [ -n "${HOME:-}" ] || die "HOME is not set"
    case "$HOME" in
        /) die "HOME cannot be the filesystem root" ;;
        /*) ;;
        *) die "HOME must be an absolute path" ;;
    esac
    case "$HOME" in
        *:*) die "HOME cannot contain ':' because it is used in PATH" ;;
    esac
}

read_tty() {
    [ -r /dev/tty ] || die "Interactive selection requires a terminal. Pass --host and --scope instead."
    IFS= read -r "$1" </dev/tty
}

choose_host() {
    printf '%s\n' "Choose an Agent Skill host:" >/dev/tty
    printf '%s\n' "  1) Agent Skills, cross-client  (~/.agents/skills)" >/dev/tty
    printf '%s\n' "  2) Cursor  (~/.cursor/skills)" >/dev/tty
    printf '%s\n' "  3) Claude Code  (~/.claude/skills)" >/dev/tty
    printf '%s\n' "  4) Custom path" >/dev/tty
    printf '%s' "Selection [1]: " >/dev/tty
    selection=""
    read_tty selection

    case "$selection" in
        ""|1) host="agents" ;;
        2) host="cursor" ;;
        3) host="claude" ;;
        4) host="custom" ;;
        *) die "Unknown host selection: $selection" ;;
    esac
}

choose_scope() {
    printf '%s\n' "Choose an installation scope:" >/dev/tty
    printf '%s\n' "  1) User" >/dev/tty
    printf '%s\n' "  2) Project" >/dev/tty
    printf '%s' "Selection [1]: " >/dev/tty
    selection=""
    read_tty selection

    case "$selection" in
        ""|1) scope="user" ;;
        2) scope="project" ;;
        *) die "Unknown scope selection: $selection" ;;
    esac
}

choose_custom_root() {
    printf '%s' "Custom skill parent directory: " >/dev/tty
    read_tty custom_root
    [ -n "$custom_root" ] || die "Custom skill parent directory cannot be empty"
}

project_root() {
    git rev-parse --show-toplevel 2>/dev/null || pwd
}

resolve_skill_parent() {
    if [ -z "$host" ]; then
        choose_host
    fi

    case "$host" in
        agents|codex|cursor|claude)
            if [ "$scope" = "ask" ]; then
                choose_scope
            fi
            case "$scope" in
                user)
                    case "$host" in
                        agents|codex) printf '%s\n' "$HOME/.agents/skills" ;;
                        cursor) printf '%s\n' "$HOME/.cursor/skills" ;;
                        claude) printf '%s\n' "$HOME/.claude/skills" ;;
                    esac
                    ;;
                project)
                    case "$host" in
                        agents|codex) printf '%s/.agents/skills\n' "$(project_root)" ;;
                        cursor) printf '%s/.cursor/skills\n' "$(project_root)" ;;
                        claude) printf '%s/.claude/skills\n' "$(project_root)" ;;
                    esac
                    ;;
                *) die "Unknown scope: $scope" ;;
            esac
            ;;
        custom)
            if [ -z "$custom_root" ]; then
                choose_custom_root
            fi
            printf '%s\n' "$custom_root"
            ;;
        *) die "Unknown host: $host" ;;
    esac
}

normalize_skill_parent() {
    path="$1"
    case "$path" in
        "~") path="$HOME" ;;
        "~/"*) path="$HOME/${path#\~/}" ;;
        "~"*) die "Custom paths only support ~ or ~/ as home-directory shortcuts" ;;
    esac

    case "$path" in
        /*) ;;
        *) path="$(pwd -P)/$path" ;;
    esac
    printf '%s\n' "$path"
}

resolve_data_home() {
    if [ -z "${XDG_DATA_HOME:-}" ]; then
        printf '%s\n' "$HOME/.local/share"
        return
    fi

    case "$XDG_DATA_HOME" in
        /) die "XDG_DATA_HOME cannot be the filesystem root" ;;
        /*) printf '%s\n' "${XDG_DATA_HOME%/}" ;;
        *)
            printf '%s\n' "Warning: ignoring relative XDG_DATA_HOME: $XDG_DATA_HOME" >&2
            printf '%s\n' "$HOME/.local/share"
            ;;
    esac
}

download() {
    url="$1"
    output="$2"

    case "$url" in
        file://*)
            cp "${url#file://}" "$output"
            return
            ;;
    esac

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --retry 3 --output "$output" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --tries=3 --output-document="$output" "$url"
    else
        die "Install curl or wget, then retry"
    fi
}

verify_platform() {
    case "$(uname -s)" in
        Linux) ;;
        *) die "This installer supports Linux only" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64) ;;
        *) die "No release artifact is available for architecture: $(uname -m)" ;;
    esac
}

confirm() {
    [ "$assume_yes" = "true" ] && return

    printf '%s\n' "Install $ARCHIVE runtime at $runtime and Skill at $destination?" >/dev/tty
    printf '%s' "Continue? [y/N] " >/dev/tty
    answer=""
    read_tty answer
    case "$answer" in
        y|Y|yes|YES) ;;
        *) die "Installation cancelled" ;;
    esac
}

backup_path() {
    original="$1"
    backup="${original}.backup.$(date +%Y%m%d%H%M%S)"
    while [ -e "$backup" ] || [ -L "$backup" ]; do
        backup="${backup}.new"
    done
    printf '%s\n' "$backup"
}

backup_existing() {
    original="$1"
    label="$2"
    backup="$(backup_path "$original")"
    mv "$original" "$backup"
    printf '%s\n' "$label preserved at $backup"
}

set_managed_paths() {
    data_home="$(resolve_data_home)"
    application_root="$data_home/swe-grep"
    runtime="$application_root/runtime"
    registry_dir="$application_root/installations"
    launcher_dir="$HOME/.local/bin"
    launcher="$launcher_dir/swe-grep"
    launcher_target="$runtime/bin/swe-grep"
}

validate_registry_path() {
    carriage_return="$(printf '\r')"
    case "$destination" in
        *"$carriage_return"*|*'
'*) die "Skill paths cannot contain carriage returns or newlines" ;;
    esac
}

registry_entry() {
    destination_hash="$(printf '%s' "$destination" | sha256sum | awk '{print $1}')"
    printf '%s/%s\n' "$registry_dir" "$destination_hash"
}

record_installation() {
    validate_registry_path
    entry="$(registry_entry)"
    skill_hash="$(sha256sum "$destination/SKILL.md" | awk '{print $1}')"
    mkdir -p "$registry_dir"
    printf '%s\n%s\n' "$destination" "$skill_hash" > "$entry"
}

registry_has_other_installations() {
    current_entry="$1"
    [ -d "$registry_dir" ] || return 1
    for registered_entry in "$registry_dir"/*; do
        if [ -f "$registered_entry" ] && [ "$registered_entry" != "$current_entry" ]; then
            return 0
        fi
    done
    return 1
}

preflight() {
    if [ -e "$destination" ] || [ -L "$destination" ]; then
        [ "$force" = "true" ] || die "Destination already exists: $destination (pass --force to preserve it as a backup)"
    fi

    existing_command="$(command -v swe-grep 2>/dev/null || true)"
    if [ -n "$existing_command" ] && [ "$existing_command" != "$launcher" ]; then
        die "Another swe-grep command is already available at $existing_command. Remove that PATH conflict before installing."
    fi

    if [ -L "$launcher" ] && [ "$(readlink "$launcher")" = "$launcher_target" ]; then
        return
    fi
    if [ -e "$launcher" ] || [ -L "$launcher" ]; then
        [ "$force" = "true" ] || die "Launcher already exists: $launcher (pass --force to preserve it as a backup)"
    fi
}

validate_package() {
    package="$1"
    [ -f "$package/SKILL.md" ] || die "Release archive does not contain swe-grep/SKILL.md"
    [ -f "$package/.env" ] || die "Release archive does not contain swe-grep/.env"
    [ -f "$package/include.txt" ] || die "Release archive does not contain swe-grep/include.txt"
    [ -f "$package/exclude.txt" ] || die "Release archive does not contain swe-grep/exclude.txt"
    [ -x "$package/bin/swe-grep" ] || die "Release archive does not contain an executable bin/swe-grep"
    [ -x "$package/bin/rg" ] || die "Release archive does not contain an executable bin/rg"
}

runtime_matches() {
    candidate="$1"
    [ -f "$runtime/SKILL.md" ] || return 1
    [ -f "$runtime/.env" ] || return 1
    [ -f "$runtime/include.txt" ] || return 1
    [ -f "$runtime/exclude.txt" ] || return 1
    [ -x "$runtime/bin/swe-grep" ] || return 1
    [ -x "$runtime/bin/rg" ] || return 1
    cmp -s "$runtime/SKILL.md" "$candidate/SKILL.md" || return 1
    cmp -s "$runtime/bin/swe-grep" "$candidate/bin/swe-grep" || return 1
    cmp -s "$runtime/bin/rg" "$candidate/bin/rg"
}

preserve_config() {
    source_root="$1"
    candidate="$2"
    for config_file in .env include.txt exclude.txt; do
        if [ -f "$source_root/$config_file" ]; then
            cp "$source_root/$config_file" "$candidate/$config_file"
        fi
    done
}

install_runtime() {
    candidate="$1"

    if runtime_matches "$candidate"; then
        return
    fi

    if [ -e "$runtime" ] || [ -L "$runtime" ]; then
        [ "$force" = "true" ] || die "A different swe-grep runtime is already installed at $runtime (pass --force to preserve and upgrade it)"
        preserve_config "$runtime" "$candidate"
        backup_existing "$runtime" "Existing runtime"
    elif [ "$force" = "true" ] && { [ -e "$destination" ] || [ -L "$destination" ]; }; then
        preserve_config "$destination" "$candidate"
    fi

    mkdir -p "$(dirname "$runtime")"
    mv "$candidate" "$runtime"
}

install_skill() {
    mkdir -p "$skill_parent"
    if [ -e "$destination" ] || [ -L "$destination" ]; then
        backup_existing "$destination" "Existing skill"
    fi
    mkdir "$destination"
    cp "$runtime/SKILL.md" "$destination/SKILL.md"
}

install_launcher() {
    mkdir -p "$launcher_dir"
    if [ -L "$launcher" ] && [ "$(readlink "$launcher")" = "$launcher_target" ]; then
        return
    fi
    if [ -e "$launcher" ] || [ -L "$launcher" ]; then
        backup_existing "$launcher" "Existing launcher"
    fi
    ln -s "$launcher_target" "$launcher"
}

add_local_bin_to_profile() {
    profile="$HOME/.profile"
    marker="# >>> swe-grep installer >>>"
    end_marker="# <<< swe-grep installer <<<"

    touch "$profile"
    if grep -Fq "$marker" "$profile"; then
        grep -Fq "$end_marker" "$profile" || die "The swe-grep block in ~/.profile is incomplete"
        return
    fi
    if grep -Fq "$end_marker" "$profile"; then
        die "The swe-grep block in ~/.profile is incomplete"
    fi

    printf '\n%s\n' "$marker" >> "$profile"
    printf '%s\n' 'if [ -d "$HOME/.local/bin" ]; then' >> "$profile"
    printf '%s\n' '  case ":${PATH:-}:" in' >> "$profile"
    printf '%s\n' '    *":$HOME/.local/bin:"*|*":$HOME/.local/bin/:"*) ;;' >> "$profile"
    printf '%s\n' '    *) PATH="$HOME/.local/bin${PATH:+:$PATH}"; export PATH ;;' >> "$profile"
    printf '%s\n' '  esac' >> "$profile"
    printf '%s\n' 'fi' >> "$profile"
    printf '%s\n' "$end_marker" >> "$profile"
}

remove_local_bin_from_profile() {
    mode="${1:-remove}"
    profile="$HOME/.profile"
    marker="# >>> swe-grep installer >>>"
    end_marker="# <<< swe-grep installer <<<"
    [ -f "$profile" ] || return

    start_count="$(grep -Fxc "$marker" "$profile" || true)"
    end_count="$(grep -Fxc "$end_marker" "$profile" || true)"
    if [ "$start_count" -eq 0 ] && [ "$end_count" -eq 0 ]; then
        return
    fi
    if [ "$start_count" -ne 1 ] || [ "$end_count" -ne 1 ]; then
        printf '%s\n' "Warning: leaving ~/.profile unchanged because the swe-grep block is malformed." >&2
        return 1
    fi

    profile_check_dir="$(mktemp -d "${TMPDIR:-/tmp}/swe-grep-profile.XXXXXX")"
    expected_block="$profile_check_dir/expected"
    actual_block="$profile_check_dir/actual"
    profile_temp="$profile_check_dir/profile"
    printf '%s\n' "$marker" > "$expected_block"
    printf '%s\n' 'if [ -d "$HOME/.local/bin" ]; then' >> "$expected_block"
    printf '%s\n' '  case ":${PATH:-}:" in' >> "$expected_block"
    printf '%s\n' '    *":$HOME/.local/bin:"*|*":$HOME/.local/bin/:"*) ;;' >> "$expected_block"
    printf '%s\n' '    *) PATH="$HOME/.local/bin${PATH:+:$PATH}"; export PATH ;;' >> "$expected_block"
    printf '%s\n' '  esac' >> "$expected_block"
    printf '%s\n' 'fi' >> "$expected_block"
    printf '%s\n' "$end_marker" >> "$expected_block"
    awk -v start="$marker" -v finish="$end_marker" '
        $0 == start { copying = 1 }
        copying { print }
        copying && $0 == finish { exit }
    ' "$profile" > "$actual_block"
    if ! cmp -s "$expected_block" "$actual_block"; then
        rm -rf "$profile_check_dir"
        printf '%s\n' "Warning: leaving ~/.profile unchanged because the swe-grep block was modified." >&2
        return 1
    fi
    if [ "$mode" = "check" ]; then
        rm -rf "$profile_check_dir"
        return 0
    fi

    if ! awk -v start="$marker" -v finish="$end_marker" '
        $0 == start {
            if (held_set && held != "") print held
            held = ""
            held_set = 0
            removing = 1
            next
        }
        removing && $0 == finish { removing = 0; found = 1; next }
        removing { next }
        {
            if (held_set) print held
            held = $0
            held_set = 1
        }
        END {
            if (removing || !found) exit 1
            if (held_set) print held
        }
    ' "$profile" > "$profile_temp"; then
        rm -rf "$profile_check_dir"
        printf '%s\n' "Warning: leaving ~/.profile unchanged because the swe-grep block could not be parsed." >&2
        return 1
    fi
    cp "$profile_temp" "$profile"
    rm -rf "$profile_check_dir"
}

confirm_uninstall() {
    [ "$assume_yes" = "true" ] && return
    printf '%s\n' "Remove the managed Agent Skill at $destination?" >/dev/tty
    printf '%s' "Continue? [y/N] " >/dev/tty
    answer=""
    read_tty answer
    case "$answer" in
        y|Y|yes|YES) ;;
        *) die "Uninstallation cancelled" ;;
    esac
}

uninstall_managed() {
    validate_registry_path
    entry="$(registry_entry)"
    [ -f "$entry" ] || die "The selected Skill is not recorded as a swe-grep managed installation: $destination"

    recorded_destination="$(sed -n '1p' "$entry")"
    recorded_hash="$(sed -n '2p' "$entry")"
    [ "$recorded_destination" = "$destination" ] || die "The installation record does not match: $destination"
    [ -n "$recorded_hash" ] || die "The installation record is incomplete: $entry"
    has_other_installations="false"
    if registry_has_other_installations "$entry"; then
        has_other_installations="true"
    else
        remove_local_bin_from_profile check || die "The managed ~/.profile block must be restored before uninstalling the last Skill"
    fi

    skill_file="$destination/SKILL.md"
    if [ -e "$skill_file" ] || [ -L "$skill_file" ]; then
        [ -f "$skill_file" ] || die "Managed SKILL.md is not a regular file: $skill_file"
        current_hash="$(sha256sum "$skill_file" | awk '{print $1}')"
        [ "$current_hash" = "$recorded_hash" ] || die "Managed SKILL.md was modified; leaving it and the installation record unchanged: $skill_file"
        rm -f "$skill_file"
    fi
    if [ -d "$destination" ]; then
        rmdir "$destination" 2>/dev/null || printf '%s\n' "Kept non-installer files at $destination"
    fi
    if [ "$has_other_installations" = "true" ]; then
        rm -f "$entry"
        printf '%s\n' "Removed Agent Skill at $destination"
        printf '%s\n' "Shared runtime and launcher were kept because other managed Skills remain."
        return
    fi

    if [ -L "$launcher" ] && [ "$(readlink "$launcher")" = "$launcher_target" ]; then
        rm -f "$launcher"
    elif [ -e "$launcher" ] || [ -L "$launcher" ]; then
        printf '%s\n' "Warning: launcher was modified and was left unchanged: $launcher" >&2
    fi
    if [ -e "$runtime" ] || [ -L "$runtime" ]; then
        rm -rf "$runtime"
    fi
    remove_local_bin_from_profile || die "The managed ~/.profile block could not be removed"
    rm -f "$entry"
    rmdir "$registry_dir" 2>/dev/null || true
    rmdir "$launcher_dir" 2>/dev/null || true
    rmdir "$application_root" 2>/dev/null || true

    printf '%s\n' "Removed Agent Skill at $destination"
    printf '%s\n' "Removed the shared swe-grep runtime and managed launcher."
    printf '%s\n' "Log out and back in to unload the ~/.profile PATH change from existing sessions."
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --host)
            [ "$#" -ge 2 ] || die "--host requires a value"
            host="$2"
            shift 2
            ;;
        --scope)
            [ "$#" -ge 2 ] || die "--scope requires a value"
            scope="$2"
            shift 2
            ;;
        --path)
            [ "$#" -ge 2 ] || die "--path requires a value"
            custom_root="$2"
            shift 2
            ;;
        --release-base-url)
            [ "$#" -ge 2 ] || die "--release-base-url requires a value"
            release_base_url="${2%/}"
            shift 2
            ;;
        --uninstall)
            uninstall="true"
            shift
            ;;
        --force)
            force="true"
            shift
            ;;
        --yes)
            assume_yes="true"
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            die "Unknown option: $1"
            ;;
    esac
done

if [ -n "$custom_root" ] && [ "$host" != "" ] && [ "$host" != "custom" ]; then
    die "--path can only be used with --host custom"
fi
if [ "$host" = "custom" ] && [ "$scope" != "user" ]; then
    die "--scope cannot be used with --host custom"
fi
if [ -z "$host" ] && [ "$scope" = "user" ]; then
    scope="ask"
fi

validate_environment
verify_platform
require_command sha256sum
require_command awk
require_command cmp

skill_parent="$(resolve_skill_parent)"
skill_parent="$(normalize_skill_parent "$skill_parent")"
[ -n "$skill_parent" ] || die "Could not determine a skill directory"
destination="$skill_parent/swe-grep"
set_managed_paths
if [ "$uninstall" = "true" ]; then
    confirm_uninstall
    uninstall_managed
    exit 0
fi
require_command tar
preflight
confirm

temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/swe-grep-install.XXXXXX")"
cleanup() {
    rm -rf "$temp_dir"
}
trap cleanup EXIT HUP INT TERM

archive_path="$temp_dir/$ARCHIVE"
checksum_path="$temp_dir/$ARCHIVE.sha256"
download "$release_base_url/$ARCHIVE" "$archive_path"
download "$release_base_url/$ARCHIVE.sha256" "$checksum_path"

(
    cd "$temp_dir"
    sha256sum -c "$checksum_path"
)

package_root="$temp_dir/package"
mkdir "$package_root"
tar -xzf "$archive_path" -C "$package_root"
candidate="$package_root/swe-grep"
validate_package "$candidate"
install_runtime "$candidate"
install_skill
install_launcher
add_local_bin_to_profile
record_installation

printf '%s\n' "Installed swe-grep runtime at $runtime"
printf '%s\n' "Installed Agent Skill at $destination"
printf '%s\n' "Launcher available at $launcher"
case ":${PATH:-}:" in
    *":$launcher_dir:"*|*":$launcher_dir/:"*)
        printf '%s\n' "$launcher_dir is available in the current PATH and persisted through ~/.profile."
        ;;
    *)
        printf '%s\n' "Log out and back in to load the PATH change from ~/.profile."
        ;;
esac
