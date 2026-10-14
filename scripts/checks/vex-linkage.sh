#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The linkage gate: every OS VEX statement that argues from what a program
# loads and imports is checked against the program the release just built.
#
# `security/vex/ferroehr-os.openvex.json` judges the distroless glibc, gcc-14
# and zlib findings of the `ferroehr` and `ferroehr-viewer` images from the
# one ELF program in each: its DT_NEEDED list, the absence of RPATH and
# RUNPATH, and the glibc functions it never imports. Those facts were read
# once with readelf. A dependency that adds C code, links libstdc++ or zlib,
# or imports one of those functions turns a `not_affected` statement false,
# and `vex-coverage.sh` still passes, because it matches ids and products and
# never reads the argument. So each statement carries its facts as data, a
# `ferroehr:linkage` block (our own extension; OpenVEX defines none):
#
#   binaries                     the program basenames the facts are about
#   needed + loader.<arch>       the EXACT DT_NEEDED set: a library added or
#                                dropped both fail, as the prose names the set
#   rpath: false                 no DT_RPATH and no DT_RUNPATH
#   forbidden_imports            undefined dynamic symbols that must not exist
#   forbidden_import_prefixes    the same, by name prefix (`_Z`: any C++ name)
#   forbidden_version_prefixes   symbol versions no import may carry
#   forbidden_strings            byte strings the file must not contain
#
# This reads the facts with `readelf -h -d --dyn-syms -W` (GNU binutils, the
# release lane's job container ships it with the `strip` it already runs):
# the ELF header for the machine, the dynamic section for NEEDED, RPATH and
# RUNPATH, and the dynamic symbol table, where an import is an entry whose
# section index is UND (https://sourceware.org/binutils/docs/binutils/readelf.html).
# A listing with no NEEDED entry or no import is refused as unreadable rather
# than judged clean.
#
# Usage:
#   scripts/checks/vex-linkage.sh [--doc <openvex.json>] <binary>...
#   scripts/checks/vex-linkage.sh --self-test
#
# Exit codes: 0 every statement holds, 1 a statement is contradicted by a
# binary, 2 a usage error or an input this cannot judge.
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
readonly GRAMMAR="[--doc <openvex.json>] <binary>... | --self-test"
guard_known_flags "$GRAMMAR" "--doc --self-test" "$@"

die() {
  echo "error: $0: $*" >&2
  exit 2
}

# facts <binary> <out>: one "<kind> <value...>" line per fact readelf reports.
# kinds: machine <amd64|arm64|other:...>, needed <lib>, rpath <path>,
# import <name> <version or ->.
facts() {
  local binary="$1" out="$2" listing
  listing="$(LC_ALL=C readelf -h -d --dyn-syms -W "$binary")" \
    || die "readelf could not read ${binary}"
  awk '
    function bracketed(line,    s, e) {
      s = index(line, "["); e = index(line, "]")
      return (s > 0 && e > s) ? substr(line, s + 1, e - s - 1) : ""
    }
    /^ *Machine:/ {
      m = $0; sub(/^ *Machine: */, "", m)
      if (m ~ /X86-64/) print "machine amd64"
      else if (m ~ /AArch64/) print "machine arm64"
      else { gsub(/ /, "_", m); print "machine other:" m }
      next
    }
    /\(NEEDED\)/ { v = bracketed($0); if (v != "") print "needed " v; next }
    /\((RPATH|RUNPATH)\)/ { v = bracketed($0); if (v != "") print "rpath " v; next }
    /^Symbol table .\.dynsym./ { dynsym = 1; next }
    /^Symbol table / { dynsym = 0; next }
    dynsym && $1 ~ /^[0-9]+:$/ && $7 == "UND" && NF >= 8 {
      name = $8; version = "-"
      at = index(name, "@")
      if (at > 0) { version = substr(name, at); name = substr(name, 1, at - 1); sub(/^@+/, "", version) }
      print "import " name " " version
    }
  ' <<<"$listing" > "$out"
  grep -q '^needed ' "$out" || die "${binary}: readelf reports no DT_NEEDED entry; not a dynamically linked program this can judge"
  grep -q '^import ' "$out" || die "${binary}: readelf reports no imported symbol; an empty table would pass every check"
  grep -q '^machine ' "$out" || die "${binary}: readelf reports no ELF machine"
}

# judge <binary> <facts> <doc>: print one VIOLATED line per contradicted
# statement fact; return 1 when any.
judge() {
  local binary="$1" facts_file="$2" doc="$3" name arch
  name="$(basename "$binary")"
  arch="$(awk '$1 == "machine" { print $2; exit }' "$facts_file")"
  case "$arch" in
  amd64 | arm64) ;;
  *) die "${binary}: ELF machine ${arch#other:} is not one the images are published for (amd64, arm64)" ;;
  esac

  local blocks
  blocks="$(jq -c --arg name "$name" '
    .statements[] | select(.["ferroehr:linkage"] != null)
    | select(.["ferroehr:linkage"].binaries | index($name))
    | {id: .vulnerability.name} + .["ferroehr:linkage"]' "$doc")"
  [[ -n "$blocks" ]] || die "${binary}: no statement in ${doc} declares linkage facts for a program named ${name}"

  local violated=0 block id want extra missing line
  local needed_have imports versions
  needed_have="$(awk '$1 == "needed" { print $2 }' "$facts_file" | LC_ALL=C sort -u)"
  imports="$(awk '$1 == "import" { print $2 }' "$facts_file" | LC_ALL=C sort -u)"
  versions="$(awk '$1 == "import" && $3 != "-" { print $3 }' "$facts_file" | LC_ALL=C sort -u)"
  while IFS= read -r block; do
    [[ -n "$block" ]] || continue
    id="$(jq -r '.id' <<<"$block")"
    want="$(jq -r --arg arch "$arch" '(.needed + [.loader[$arch] // empty])[]' <<<"$block" | LC_ALL=C sort -u)"
    jq -e --arg arch "$arch" '.loader[$arch] != null' <<<"$block" >/dev/null \
      || { echo "VIOLATED ${id} ${name} (${arch}): the statement declares no loader for ${arch}"; violated=1; }
    extra="$(LC_ALL=C comm -13 <(printf '%s\n' "$want") <(printf '%s\n' "$needed_have"))"
    missing="$(LC_ALL=C comm -23 <(printf '%s\n' "$want") <(printf '%s\n' "$needed_have"))"
    for line in $extra; do
      echo "VIOLATED ${id} ${name} (${arch}): DT_NEEDED carries ${line}, which the statement does not declare"
      violated=1
    done
    for line in $missing; do
      echo "VIOLATED ${id} ${name} (${arch}): DT_NEEDED lacks ${line}, which the statement declares"
      violated=1
    done
    if [[ "$(jq -r '.rpath' <<<"$block")" == "false" ]]; then
      while IFS= read -r line; do
        [[ -n "$line" ]] || continue
        echo "VIOLATED ${id} ${name} (${arch}): carries RPATH/RUNPATH ${line}"
        violated=1
      done < <(awk '$1 == "rpath" { $1 = ""; sub(/^ /, ""); print }' "$facts_file")
    fi
    while IFS= read -r line; do
      [[ -n "$line" ]] || continue
      grep -qxF -- "$line" <<<"$imports" || continue
      echo "VIOLATED ${id} ${name} (${arch}): imports ${line}"
      violated=1
    done < <(jq -r '(.forbidden_imports // [])[]' <<<"$block")
    while IFS= read -r want; do
      [[ -n "$want" ]] || continue
      while IFS= read -r line; do
        [[ -n "$line" && "$line" == "$want"* ]] || continue
        echo "VIOLATED ${id} ${name} (${arch}): imports ${line} (prefix ${want})"
        violated=1
      done <<<"$imports"
    done < <(jq -r '(.forbidden_import_prefixes // [])[]' <<<"$block")
    while IFS= read -r want; do
      [[ -n "$want" ]] || continue
      while IFS= read -r line; do
        [[ -n "$line" && "$line" == "$want"* ]] || continue
        echo "VIOLATED ${id} ${name} (${arch}): imports a symbol at version ${line} (prefix ${want})"
        violated=1
      done <<<"$versions"
    done < <(jq -r '(.forbidden_version_prefixes // [])[]' <<<"$block")
    while IFS= read -r want; do
      [[ -n "$want" ]] || continue
      LC_ALL=C grep -qaF -- "$want" "$binary" || continue
      echo "VIOLATED ${id} ${name} (${arch}): contains the string '${want}'"
      violated=1
    done < <(jq -r '(.forbidden_strings // [])[]' <<<"$block")
  done <<<"$blocks"
  return "$violated"
}

check() {
  local doc="security/vex/ferroehr-os.openvex.json"
  local -a binaries=()
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
    --doc)
      [[ "$#" -ge 2 ]] || guard_usage "$GRAMMAR"
      doc="$2"
      shift 2
      ;;
    *)
      binaries+=("$1")
      shift
      ;;
    esac
  done
  [[ "${#binaries[@]}" -gt 0 ]] || guard_usage "$GRAMMAR"
  local tool
  for tool in readelf jq awk comm; do
    command -v "$tool" >/dev/null || die "${tool} is required"
  done
  [[ -f "$doc" ]] || die "${doc} does not exist"
  jq -e '.statements | type == "array"' "$doc" >/dev/null 2>&1 || die "${doc} is not an OpenVEX document"

  local work binary failed=0 count
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand now: $work is local and gone at EXIT time
  trap "rm -rf '$work'" EXIT
  for binary in "${binaries[@]}"; do
    [[ -f "$binary" ]] || die "${binary} does not exist"
    facts "$binary" "$work/facts"
    if judge "$binary" "$work/facts" "$doc"; then
      count="$(jq --arg name "$(basename "$binary")" '[.statements[] | select(.["ferroehr:linkage"].binaries // [] | index($name))] | length' "$doc")"
      echo "vex-linkage: $(basename "$binary") ($(awk '$1 == "machine" { print $2; exit }' "$work/facts")): ${count} statement(s) hold: DT_NEEDED $(awk '$1 == "needed" { printf "%s ", $2 }' "$work/facts")and no forbidden import"
    else
      failed=1
    fi
  done
  [[ "$failed" -eq 0 ]] || {
    echo "vex-linkage: a statement in ${doc} no longer describes the binary it judges." >&2
    echo "  Re-read the statement against the binary, then correct its prose and its ferroehr:linkage block together." >&2
    exit 1
  }
}

# ── The self-test: the detector, proven on synthetic readelf output ──────────
# A stub `readelf` on PATH prints a listing assembled per case; the "binary" is
# a file of chosen bytes, read only by the string check. The document is the
# real one, so every forbidden import it declares is driven through the
# detector once, which proves no entry is dead.
self_test() {
  local tmp
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand now: $tmp is local and gone at EXIT time
  trap "rm -rf '$tmp'" EXIT
  mkdir -p "$tmp/bin" "$tmp/amd64" "$tmp/arm64"
  local doc
  doc="$(cd "$(dirname "$0")/../.." && pwd)/security/vex/ferroehr-os.openvex.json"
  [[ -f "$doc" ]] || die "self-test: ${doc} does not exist"
  cat > "$tmp/bin/readelf" <<'STUB'
#!/usr/bin/env bash
cat "$READELF_FIXTURE"
STUB
  chmod +x "$tmp/bin/readelf"

  # listing <file> <machine> <needed...> -- <dynamic extra lines> -- <imports...>
  # An import is name[@VERSION].
  listing() {
    local file="$1" machine="$2" n=1
    shift 2
    {
      printf 'ELF Header:\n  Class:                             ELF64\n  Machine:                           %s\n\n' "$machine"
      printf 'Dynamic section at offset 0x1 contains 30 entries:\n  Tag        Type                         Name/Value\n'
      while [[ "$#" -gt 0 && "$1" != "--" ]]; do
        printf ' 0x0000000000000001 (NEEDED)             Shared library: [%s]\n' "$1"
        shift
      done
      shift
      while [[ "$#" -gt 0 && "$1" != "--" ]]; do
        printf '%s\n' "$1"
        shift
      done
      shift
      printf ' 0x0000000000000000 (NULL)               0x0\n\n'
      printf "Symbol table '.dynsym' contains %d entries:\n" "$(($# + 1))"
      printf '   Num:    Value          Size Type    Bind   Vis      Ndx Name\n'
      printf '     0: 0000000000000000     0 NOTYPE  LOCAL  DEFAULT  UND \n'
      local sym
      for sym in "$@"; do
        if [[ "$sym" == *@* ]]; then
          printf '%6d: 0000000000000000     0 FUNC    GLOBAL DEFAULT  UND %s (2)\n' "$n" "$sym"
        else
          printf '%6d: 0000000000000000     0 NOTYPE  WEAK   DEFAULT  UND %s\n' "$n" "$sym"
        fi
        n=$((n + 1))
      done
      printf '%6d: 0000000000123450    42 FUNC    GLOBAL DEFAULT   14 glob_defined_here\n' "$n"
    } > "$file"
  }
  local amd64_ok=(libgcc_s.so.1 libm.so.6 libc.so.6 ld-linux-x86-64.so.2)
  local base_imports=(__libc_start_main@GLIBC_2.34 malloc@GLIBC_2.2.5 getaddrinfo@GLIBC_2.2.5
    dlsym@GLIBC_2.34 _Unwind_Resume@GCC_3.0 __gmon_start__ __cxa_finalize@GLIBC_2.2.5)

  local failed=0
  # run <label> <expected exit> <expected output fragment or -> <binary name> <fixture>
  run() {
    local label="$1" status="$2" want="$3" name="$4" fixture="$5" rc=0 out
    mkdir -p "$tmp/case"
    [[ -f "$tmp/case/$name" ]] || printf 'synthetic ELF bytes\n' > "$tmp/case/$name"
    out="$(PATH="$tmp/bin:$PATH" READELF_FIXTURE="$fixture" bash "$0" --doc "$doc" "$tmp/case/$name" 2>&1)" || rc=$?
    rm -rf "$tmp/case"
    if [[ "$rc" -ne "$status" ]]; then
      echo "self-test FAILED: ${label}: exit ${rc}, expected ${status}" >&2
      printf '%s\n' "$out" | head -8 | sed 's/^/    /' >&2
      failed=1
    elif [[ "$want" != "-" && "$out" != *"$want"* ]]; then
      echo "self-test FAILED: ${label}: output does not name '${want}'" >&2
      printf '%s\n' "$out" | head -8 | sed 's/^/    /' >&2
      failed=1
    else
      echo "  self-test: ${label}"
    fi
  }

  local f="$tmp/listing"
  listing "$f" "Advanced Micro Devices X86-64" "${amd64_ok[@]}" -- -- "${base_imports[@]}"
  run "the declared amd64 linkage holds" 0 "statement(s) hold" ferroehr "$f"
  run "the viewer is judged by the same statements" 0 "statement(s) hold" ferroehr-viewer "$f"
  listing "$f" "AArch64" libgcc_s.so.1 libm.so.6 libc.so.6 ld-linux-aarch64.so.1 -- -- "${base_imports[@]}"
  run "the declared arm64 linkage holds" 0 "statement(s) hold" ferroehr "$f"
  listing "$f" "AArch64" "${amd64_ok[@]}" -- -- "${base_imports[@]}"
  run "the amd64 loader on arm64 is refused" 1 "DT_NEEDED carries ld-linux-x86-64.so.2" ferroehr "$f"
  listing "$f" "Advanced Micro Devices X86-64" "${amd64_ok[@]}" libz.so.1 -- -- "${base_imports[@]}"
  run "an added library (zlib) is refused" 1 "VIOLATED CVE-2026-27171 ferroehr (amd64): DT_NEEDED carries libz.so.1" ferroehr "$f"
  listing "$f" "Advanced Micro Devices X86-64" "${amd64_ok[@]}" libstdc++.so.6 -- -- "${base_imports[@]}"
  run "an added library (libstdc++) is refused" 1 "VIOLATED CVE-2026-102010 ferroehr (amd64): DT_NEEDED carries libstdc++.so.6" ferroehr "$f"
  listing "$f" "Advanced Micro Devices X86-64" libgcc_s.so.1 libc.so.6 ld-linux-x86-64.so.2 -- -- "${base_imports[@]}"
  run "a dropped library is refused" 1 "DT_NEEDED lacks libm.so.6" ferroehr "$f"
  # shellcheck disable=SC2016 # $ORIGIN is the literal loader token a RUNPATH carries
  listing "$f" "Advanced Micro Devices X86-64" "${amd64_ok[@]}" \
    -- ' 0x000000000000001d (RUNPATH)            Library runpath: [$ORIGIN/../lib]' -- "${base_imports[@]}"
  run "a RUNPATH is refused" 1 "VIOLATED CVE-2026-95818 ferroehr (amd64): carries RPATH/RUNPATH \$ORIGIN/../lib" ferroehr "$f"
  listing "$f" "Advanced Micro Devices X86-64" "${amd64_ok[@]}" -- -- "${base_imports[@]}" _ZdlPvm@GLIBCXX_3.4.21
  run "a C++ import is refused by name" 1 "VIOLATED CVE-2026-95619 ferroehr (amd64): imports _ZdlPvm (prefix _Z)" ferroehr "$f"
  run "a C++ import is refused by version" 1 "imports a symbol at version GLIBCXX_3.4.21 (prefix GLIBCXX_)" ferroehr "$f"
  listing "$f" "Advanced Micro Devices X86-64" "${amd64_ok[@]}" -- -- "${base_imports[@]}" glob64
  run "an unversioned forbidden import is refused" 1 "VIOLATED CVE-2010-4756 ferroehr (amd64): imports glob64" ferroehr "$f"
  listing "$f" "Advanced Micro Devices X86-64" "${amd64_ok[@]}" -- -- "${base_imports[@]}" globfree@GLIBC_2.2.5
  run "a name that only starts like a forbidden one is accepted" 0 "statement(s) hold" ferroehr "$f"
  listing "$f" "IBM S/390" "${amd64_ok[@]}" -- -- "${base_imports[@]}"
  run "an unpublished machine is a usage error" 2 "is not one the images are published for" ferroehr "$f"
  listing "$f" "Advanced Micro Devices X86-64" "${amd64_ok[@]}" -- --
  run "a listing with no import is refused as unreadable" 2 "reports no imported symbol" ferroehr "$f"
  listing "$f" "Advanced Micro Devices X86-64" -- -- "${base_imports[@]}"
  run "a static program is refused as unreadable" 2 "reports no DT_NEEDED entry" ferroehr "$f"
  listing "$f" "Advanced Micro Devices X86-64" "${amd64_ok[@]}" -- -- "${base_imports[@]}"
  run "a program no statement describes is a usage error" 2 "declares linkage facts for a program named ferroehr-postgres" ferroehr-postgres "$f"
  mkdir -p "$tmp/case"
  printf 'x deflate 1.3.1 Copyright 1995-2024 Jean-loup Gailly and Mark Adler x\n' > "$tmp/case/ferroehr"
  run "a statically linked zlib is refused by its copyright string" 1 "VIOLATED CVE-2026-85091 ferroehr (amd64): contains the string 'Jean-loup Gailly'" ferroehr "$f"

  # Every forbidden import the document declares, one at a time, with its
  # version when the statement could see one: each must fail and name a
  # statement that declares it.
  local sym ids
  while IFS= read -r sym; do
    [[ -n "$sym" ]] || continue
    ids="$(jq -r --arg s "$sym" '[.statements[] | select((.["ferroehr:linkage"].forbidden_imports // []) | index($s)) | .vulnerability.name] | first' "$doc")"
    listing "$f" "Advanced Micro Devices X86-64" "${amd64_ok[@]}" -- -- "${base_imports[@]}" "${sym}@GLIBC_2.2.5"
    run "the declared forbidden import ${sym} is detected" 1 "VIOLATED ${ids} ferroehr (amd64): imports ${sym}" ferroehr "$f"
  done < <(jq -r '[.statements[]["ferroehr:linkage"].forbidden_imports // [] | .[]] | unique[]' "$doc")

  local rc=0
  bash "$0" --bogus >/dev/null 2>&1 || rc=$?
  if [[ "$rc" -ne 2 ]]; then
    echo "self-test FAILED: an unknown flag exited ${rc}, expected 2" >&2
    failed=1
  else
    echo "  self-test: an unknown flag is a usage error"
  fi

  [[ "$failed" -eq 0 ]] || exit 1
  echo "vex-linkage: self-test passed"
}

if [[ "${1:-}" == "--self-test" ]]; then
  [[ "$#" -eq 1 ]] || guard_usage "$GRAMMAR"
  self_test
else
  check "$@"
fi
