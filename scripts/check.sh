#!/usr/bin/env bash
# Every check of STYLE.md, the way CI runs them. The same file is in every repository that follows STYLE.md; it finds out
# what it is checking:
#   - a Cargo workspace (Cargo.toml): rustfmt, clippy, rustdoc, cargo-machete, cargo-deny, cargo-about;
#   - a web page (biome.json): Biome, html-validate and the vendored code's licences;
#   - always: typos, Vale, markdownlint, lychee, REUSE, gitleaks and trufflehog.
# A folder below this one with its own scripts/check.sh is a separate project: it is checked by its own script, run at the end.
#
# Usage: scripts/check.sh [--online] [--pedantic]
#   --online    lychee also checks links to the web (needs the network; set GITHUB_TOKEN to avoid GitHub's rate limit)
#   --pedantic  also print clippy's pedantic lints, counted by lint (a report: it never fails)
# Exit status: 0 when every check passes, 1 otherwise. A missing tool is a failure; a different version is a warning.

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

online=false
pedantic=false
for arg in "$@"; do
    case "$arg" in
        --online) online=true ;;
        --pedantic) pedantic=true ;;
        *) echo "unknown option $arg (see the top of $0)" >&2; exit 2 ;;
    esac
done

# The versions STYLE.md pins ("The checks"). Change them there and here together.
declare -A PIN=(
    [cargo-deny]=0.20.2 [cargo-about]=0.9.3 [cargo-machete]=0.9.2
    [biome]=2.6.6 [html-validate]=11.21.0
    [typos]=1.52.0 [vale]=3.26.0 [markdownlint-cli2]=0.24.0 [lychee]=0.24.2
    [reuse]=6.3.0 [gitleaks]=8.30.1 [trufflehog]=3.99.8
)

failed=()
step() { printf '\n== %s\n' "$1"; }
fail() { failed+=("$1"); printf '   FAILED: %s\n' "$1"; }
have() {
    if ! command -v "$1" > /dev/null; then
        fail "$1 is not installed (STYLE.md, \"The checks\", says where to get it)"
        return 1
    fi
    local want=${PIN[$1]:-} got
    got=$("$1" --version 2>&1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
    if [[ -n "$want" && "$got" != "$want" ]]; then
        printf '   note: %s is %s; STYLE.md pins %s\n' "$1" "${got:-?}" "$want"
    fi
}
run() { local name=$1; shift; "$@" || fail "$name"; }

# Folders checked by their own script, and files that are not ours (vendored code, licence texts).
nested=()
while IFS= read -r f; do nested+=("${f%/scripts/check.sh}/"); done < <(git ls-files '*/scripts/check.sh')
ours() {
    local f n
    while IFS= read -r f; do
        case "$f" in vendor/* | */vendor/* | LICENSES/* | LICENSE) continue ;; esac
        for n in "${nested[@]}"; do [[ "$f" == "$n"* ]] && continue 2; done
        [[ -f "$f" ]] && printf '%s\n' "$f"
    done < <(git ls-files "$@")
}

# --- Rust -----------------------------------------------------------------------------------------------------------------
if [[ -f Cargo.toml ]]; then
    step "rustfmt"
    run rustfmt cargo fmt --all --check
    step "clippy (the shared lint set in Cargo.toml)"
    run clippy cargo clippy --workspace --all-targets --locked -- -D warnings
    if $pedantic; then
        step "clippy pedantic (report only)"
        # A hit in code shared by several targets (the library and its tests) is counted once per target.
        cargo clippy --workspace --all-targets --locked --message-format=json -- -W clippy::pedantic 2> /dev/null |
            grep -oE '"code":\{"code":"clippy::[a-z_]+"' | cut -d'"' -f6 | sort | uniq -c | sort -rn
    fi
    step "rustdoc"
    run rustdoc env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --quiet
    step "cargo-machete (unused dependencies)"
    have cargo-machete && run cargo-machete cargo machete
    step "cargo-deny (licences, advisories, duplicates, sources)"
    have cargo-deny && run cargo-deny cargo deny --locked check
    if [[ -f about.toml && -f about.hbs ]]; then
        step "cargo-about (every dependency's licence can be shipped)"
        have cargo-about && run cargo-about cargo about generate --workspace --all-features --fail --locked -o /dev/null about.hbs
    fi
fi

# --- Web ------------------------------------------------------------------------------------------------------------------
if [[ -f biome.json ]]; then
    step "Biome (format and lint: JavaScript, CSS, JSON)"
    have biome && run biome biome ci --reporter=summary .
    if [[ -f .htmlvalidate.json ]]; then
        step "html-validate"
        mapfile -t html < <(ours '*.html')
        have html-validate && ((${#html[@]})) && run html-validate html-validate "${html[@]}"
    fi
    if [[ -d vendor ]]; then
        step "vendored code: a licence, a REUSE entry and a README line for each folder"
        for d in vendor/*/; do
            d=${d%/}
            ls "$d"/LICENSE* > /dev/null 2>&1 || fail "$d has no LICENSE file"
            grep -q "\"$d/\*\*\"" REUSE.toml 2> /dev/null || fail "$d has no entry in REUSE.toml"
            grep -q "\`$d\`" README.md || fail "$d is not listed in README.md (name, version, licence)"
        done
    fi
fi

# --- Text: spelling, writing, Markdown, links ----------------------------------------------------------------------------
mapfile -t md < <(ours '*.md')
step "typos (spelling)"
have typos && run typos typos --force-exclude --file-list <(ours)
step "Vale (writing: Markdown and the comments in Rust and JavaScript)"
mapfile -t prose < <(ours '*.md' '*.rs' '*.js')
have vale && ((${#prose[@]})) && run vale vale --no-wrap "${prose[@]}"
step "markdownlint"
have markdownlint-cli2 && ((${#md[@]})) && run markdownlint markdownlint-cli2 "${md[@]}"
step "lychee (links$($online || echo ' between our own files; --online for the web'))"
if have lychee && ((${#md[@]})); then
    if $online; then run lychee lychee --no-progress "${md[@]}"; else run lychee lychee --no-progress --offline "${md[@]}"; fi
fi

# --- Licences and secrets -------------------------------------------------------------------------------------------------
# A nested project is checked again by the outer one: its licences and secrets are part of the outer repository.
if [[ -z "${CHECK_NESTED:-}" ]]; then
    step "REUSE (every file has its copyright and licence)"
    # --root: in a folder of a larger repository (a nested project), lint the folder, not the whole repository.
    have reuse && run reuse reuse --root . lint --quiet
    step "gitleaks (the tree, then every commit)"
    if have gitleaks; then
        run "gitleaks (tree)" gitleaks dir --redact --no-banner .
        run "gitleaks (history)" gitleaks git --redact --no-banner .
    fi
    step "trufflehog (the tree, then every commit)"
    if have trufflehog; then
        run "trufflehog (tree)" trufflehog filesystem --no-update --fail --results=verified,unknown . \
            --exclude-paths=<(printf '%s\n' '^target/' '/target/' '^\.git/')
        # The repository's top: trufflehog clones it, so a folder inside it will not do.
        run "trufflehog (history)" trufflehog git --no-update --fail --results=verified,unknown "file://$(git rev-parse --show-toplevel)"
    fi
fi

# --- Nested projects ------------------------------------------------------------------------------------------------------
for n in "${nested[@]}"; do
    step "${n%/}: its own checks"
    # Shared files must stay identical to this repository's copies.
    while IFS= read -r f; do
        [[ -f "$n$f" ]] && ! cmp -s "$f" "$n$f" && fail "$n$f differs from $f (a shared file: change both)"
    done < <(git ls-files STYLE.md rustfmt.toml deny.toml about.toml .editorconfig _typos.toml .vale.ini '.vale/*' \
        .markdownlint.jsonc lychee.toml scripts/check.sh)
    args=()
    $online && args+=(--online)
    $pedantic && args+=(--pedantic)
    CHECK_NESTED=1 "$n/scripts/check.sh" "${args[@]}" || fail "${n%/}"
done

echo
if ((${#failed[@]})); then
    printf 'Failed (%d):\n' "${#failed[@]}"
    printf '  - %s\n' "${failed[@]}"
    exit 1
fi
echo "Every check passed."
