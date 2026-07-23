#!/bin/sh

set -eu

[ "$#" -eq 1 ] || {
    printf '%s\n' "Usage: test-install-linux.sh RELEASE_DIRECTORY" >&2
    exit 2
}

release_dir="$(CDPATH= cd "$1" && pwd -P)"
script_dir="$(CDPATH= cd "$(dirname "$0")" && pwd -P)"
installer="$script_dir/install.sh"
archive="swe-grep-linux-x86_64-musl.tar.gz"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/swe-grep-installer-test.XXXXXX")"
cleanup() {
    rm -rf "$test_root"
}
trap cleanup EXIT HUP INT TERM

home="$test_root/home"
base_path="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
mkdir -p "$home"
printf '%s\n' 'ORIGINAL_PROFILE_VALUE=preserved' > "$home/.profile"
cp "$home/.profile" "$test_root/original.profile"

run_installer() {
    env -u XDG_DATA_HOME HOME="$home" PATH="$base_path" \
        sh "$installer" --release-base-url "file://$release_dir" --yes "$@"
}

run_uninstaller() {
    env -u XDG_DATA_HOME HOME="$home" PATH="$base_path" \
        sh "$installer" --uninstall --yes "$@"
}

run_installer --host agents --scope user
runtime="$home/.local/share/swe-grep/runtime"
launcher="$home/.local/bin/swe-grep"
test -x "$runtime/bin/swe-grep"
test -x "$runtime/bin/rg"
test "$(readlink "$launcher")" = "$runtime/bin/swe-grep"
test -f "$home/.agents/skills/swe-grep/SKILL.md"
test "$(find "$home/.agents/skills/swe-grep" -type f | wc -l)" -eq 1
grep -Fq '# >>> swe-grep installer >>>' "$home/.profile"
grep -Fq '# <<< swe-grep installer <<<' "$home/.profile"
grep -Fq 'ORIGINAL_PROFILE_VALUE=preserved' "$home/.profile"
env PATH="" "$launcher" --help

launcher_target="$(readlink "$launcher")"
run_installer --host cursor --scope user
run_installer --host claude --scope user
run_installer --host custom --path "$test_root/custom-skills"
mkdir "$test_root/project"
(
    cd "$test_root/project"
    run_installer --host agents --scope project
)
test -f "$home/.cursor/skills/swe-grep/SKILL.md"
test -f "$home/.claude/skills/swe-grep/SKILL.md"
test -f "$test_root/custom-skills/swe-grep/SKILL.md"
test -f "$test_root/project/.agents/skills/swe-grep/SKILL.md"
test "$(readlink "$launcher")" = "$launcher_target"

path_home="$test_root/path-home"
mkdir -p "$path_home"
printf '%s\n' 'PATH_PROFILE_VALUE=preserved' > "$path_home/.profile"
env XDG_DATA_HOME="relative-data-home" HOME="$path_home" PATH="$path_home/.local/bin:$base_path" \
    sh "$installer" --host custom --path "$test_root/path-skills" \
    --release-base-url "file://$release_dir" --yes
grep -Fq '# >>> swe-grep installer >>>' "$path_home/.profile"
printf '%s\n' '# user modification' >> "$test_root/path-skills/swe-grep/SKILL.md"
if env -u XDG_DATA_HOME HOME="$path_home" PATH="$base_path" \
    sh "$installer" --uninstall --host custom --path "$test_root/path-skills" --yes; then
    printf '%s\n' "Expected modified SKILL.md to block uninstallation" >&2
    exit 1
fi
cp "$path_home/.local/share/swe-grep/runtime/SKILL.md" "$test_root/path-skills/swe-grep/SKILL.md"
cp "$path_home/.profile" "$path_home/.profile.clean"
sed '/# >>> swe-grep installer >>>/a USER_BLOCK_VALUE=preserved' \
    "$path_home/.profile.clean" > "$path_home/.profile"
if env -u XDG_DATA_HOME HOME="$path_home" PATH="$base_path" \
    sh "$installer" --uninstall --host custom --path "$test_root/path-skills" --yes; then
    printf '%s\n' "Expected a modified profile block to block final uninstallation" >&2
    exit 1
fi
test -f "$test_root/path-skills/swe-grep/SKILL.md"
test -x "$path_home/.local/share/swe-grep/runtime/bin/swe-grep"
grep -Fq 'USER_BLOCK_VALUE=preserved' "$path_home/.profile"
cp "$path_home/.profile.clean" "$path_home/.profile"
env -u XDG_DATA_HOME HOME="$path_home" PATH="$base_path" \
    sh "$installer" --uninstall --host custom --path "$test_root/path-skills" --yes
test ! -e "$path_home/.local/share/swe-grep"
test ! -e "$path_home/.local/bin/swe-grep"
grep -Fq 'PATH_PROFILE_VALUE=preserved' "$path_home/.profile"
if grep -Fq 'swe-grep installer' "$path_home/.profile"; then
    printf '%s\n' "Uninstallation left its managed profile block" >&2
    exit 1
fi

conflict_home="$test_root/conflict-home"
conflict_bin="$test_root/conflict-bin"
mkdir -p "$conflict_home" "$conflict_bin"
cp /bin/true "$conflict_bin/swe-grep"
if env -u XDG_DATA_HOME HOME="$conflict_home" PATH="$conflict_bin:$base_path" \
    sh "$installer" --host agents --scope user \
    --release-base-url "file://$release_dir" --yes; then
    printf '%s\n' "Expected external command conflict" >&2
    exit 1
fi
test ! -e "$conflict_home/.agents/skills/swe-grep"
test ! -e "$conflict_home/.local/share/swe-grep/runtime"
test ! -e "$conflict_home/.profile"

printf '%s\n' 'CUSTOM_CONFIG=kept' > "$runtime/.env"
upgrade_release="$test_root/upgrade-release"
mkdir -p "$upgrade_release/package"
tar -xzf "$release_dir/$archive" -C "$upgrade_release/package"
printf '\n%s\n' '# installer upgrade test' >> "$upgrade_release/package/swe-grep/SKILL.md"
tar -czf "$upgrade_release/$archive" -C "$upgrade_release/package" swe-grep
(
    cd "$upgrade_release"
    sha256sum "$archive" > "$archive.sha256"
)

if env -u XDG_DATA_HOME HOME="$home" PATH="$home/.local/bin:$base_path" \
    sh "$installer" --host custom --path "$test_root/upgrade-skills" \
    --release-base-url "file://$upgrade_release" --yes; then
    printf '%s\n' "Expected runtime upgrade to require --force" >&2
    exit 1
fi
env -u XDG_DATA_HOME HOME="$home" PATH="$home/.local/bin:$base_path" \
    sh "$installer" --host custom --path "$test_root/upgrade-skills" \
    --release-base-url "file://$upgrade_release" --force --yes
grep -Fq 'CUSTOM_CONFIG=kept' "$runtime/.env"
find "$(dirname "$runtime")" -maxdepth 1 -name 'runtime.backup.*' | grep -q .

env -u XDG_DATA_HOME HOME="$home" PATH="$home/.local/bin:$base_path" \
    sh "$installer" --host custom --path "$test_root/upgrade-skills" \
    --release-base-url "file://$upgrade_release" --force --yes
find "$test_root/upgrade-skills" -maxdepth 1 -name 'swe-grep.backup.*' | grep -q .

missing_project="$test_root/missing-project"
if env PATH="" "$launcher" search dummy_query --path "$missing_project" \
    2> "$test_root/search.stderr"; then
    printf '%s\n' "Expected missing project search to fail" >&2
    exit 1
fi
grep -q 'Project path does not exist' "$test_root/search.stderr"
if grep -q 'ripgrep' "$test_root/search.stderr"; then
    printf '%s\n' "Launcher failed to resolve bundled rg" >&2
    exit 1
fi

printf '%s\n' 'USER_FILE=preserved' > "$test_root/custom-skills/swe-grep/user-file.txt"
run_uninstaller --host cursor --scope user
test -x "$runtime/bin/swe-grep"
test -L "$launcher"
grep -Fq '# >>> swe-grep installer >>>' "$home/.profile"
run_uninstaller --host agents --scope user
run_uninstaller --host claude --scope user
run_uninstaller --host custom --path "$test_root/custom-skills"
test -f "$test_root/custom-skills/swe-grep/user-file.txt"
test ! -e "$test_root/custom-skills/swe-grep/SKILL.md"
(
    cd "$test_root/project"
    run_uninstaller --host agents --scope project
)
run_uninstaller --host custom --path "$test_root/upgrade-skills"
test ! -e "$runtime"
test ! -e "$launcher"
grep -Fq 'ORIGINAL_PROFILE_VALUE=preserved' "$home/.profile"
if grep -Fq 'swe-grep installer' "$home/.profile"; then
    printf '%s\n' "Final uninstallation left its managed profile block" >&2
    exit 1
fi
cmp -s "$test_root/original.profile" "$home/.profile"

printf '%s\n' "Linux installer functional tests passed"
