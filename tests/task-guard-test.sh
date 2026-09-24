#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
task_command=$(python3 -c "import json, sys; print(json.load(open(sys.argv[1]))[0][\"command\"])" "$repo_root/.zed/tasks.json")
expected_stderr=$(printf "%s\\n" \
"ERROR: zed-http was not found on PATH. It is required to run HTTP requests." \
"Install it from the repository checkout root with: cargo install --path cli --locked" \
"Documentation: https://github.com/JimPr/http-client#install-the-cli" \
"No request was sent.")

workspace=$(mktemp -d)
trap "rm -rf \"$workspace\"" EXIT HUP INT TERM

# Fake-free PATH: absence must fail without invoking a client or sending a request.
mkdir "$workspace/empty-bin"
set +e
PATH="$workspace/empty-bin" ZED_FILE="/tmp/request file.http" ZED_ROW="12" /bin/sh -c "$task_command" >"$workspace/missing.stdout" 2>"$workspace/missing.stderr"
missing_status=$?
set -e
test "$missing_status" -eq 127
test ! -s "$workspace/missing.stdout"
test "$(cat "$workspace/missing.stderr")" = "$expected_stderr"

# Fake CLI records its exact arguments and returns a non-zero status unchanged.
mkdir "$workspace/fake-bin"
printf "%s\n" '#!/bin/sh' 'printf "%s\n" "$@" > "$FAKE_ARGUMENTS_FILE"' 'exit 23' > "$workspace/fake-bin/zed-http"
chmod +x "$workspace/fake-bin/zed-http"
set +e
PATH="$workspace/fake-bin" FAKE_ARGUMENTS_FILE="$workspace/arguments" ZED_FILE="/tmp/request file.http" ZED_ROW="12" /bin/sh -c "$task_command" >"$workspace/present.stdout" 2>"$workspace/present.stderr"
present_status=$?
set -e
test "$present_status" -eq 23
test ! -s "$workspace/present.stderr"
test "$(cat "$workspace/arguments")" = "$(printf "%s\\n" "/tmp/request file.http" "--line" "13")"

printf "%s\\n" "Task guard tests passed."
