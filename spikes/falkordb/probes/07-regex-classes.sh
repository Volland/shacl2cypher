#!/usr/bin/env bash
# Probe batch 7: regex class rendering for FalkorDB (Oniguruma): astral ranges up to
# U+10FFFF, empty-class replacement, mixed BMP/astral ranges, the XML name-character
# class, and whether the substring wrapper is needed. Tokens: %%BS%% -> backslash.
set -u
PODMAN=${PODMAN:-/opt/podman/bin/podman}
q() {
  printf '\n>> %s\n' "$1"
  "$PODMAN" exec s2c-falkordb redis-cli GRAPH.RO_QUERY probe2 "$1" 2>&1
}
while IFS= read -r line; do
  line=${line//%%BS%%/\\}
  case "$line" in
    '') ;;
    '#'*) printf '\n==== %s\n' "$line" ;;
    *) q "$line" ;;
  esac
done <<'PROBES'
# astral literals and U+10FFFF
RETURN string.matchRegEx('😀', '[𐀀-􏿿]'), string.matchRegEx('a', '[𐀀-􏿿]'), string.matchRegEx('𐀀', '^[%%BS%%%%BS%%u0000-%%BS%%%%BS%%uFFFF𐀀-􏿿]$')
RETURN string.matchRegEx('x', '[^%%BS%%%%BS%%u0000-%%BS%%%%BS%%uFFFF𐀀-􏿿]'), string.matchRegEx('😀', '[^%%BS%%%%BS%%u0000-%%BS%%%%BS%%uFFFF𐀀-􏿿]')
RETURN string.matchRegEx('x', '(?!)'), string.matchRegEx('x', 'a(?!)|x')
RETURN string.matchRegEx('😀', '[%%BS%%%%BS%%uFFFF-😀]'), string.matchRegEx('%%BS%%uFFFE', '[%%BS%%%%BS%%uFFFF-😀]')
# XML name start class (from xsd_regex NAME_START) in FalkorDB syntax
RETURN string.matchRegEx('é', '^[:A-Z_a-z%%BS%%%%BS%%u00C0-%%BS%%%%BS%%u00D6%%BS%%%%BS%%u00D8-%%BS%%%%BS%%u00F6%%BS%%%%BS%%u00F8-%%BS%%%%BS%%u02FF%%BS%%%%BS%%u0370-%%BS%%%%BS%%u037D%%BS%%%%BS%%u037F-%%BS%%%%BS%%u1FFF%%BS%%%%BS%%u200C-%%BS%%%%BS%%u200D%%BS%%%%BS%%u2070-%%BS%%%%BS%%u218F%%BS%%%%BS%%u2C00-%%BS%%%%BS%%u2FEF%%BS%%%%BS%%u3001-%%BS%%%%BS%%uD7FF%%BS%%%%BS%%uF900-%%BS%%%%BS%%uFDCF%%BS%%%%BS%%uFDF0-%%BS%%%%BS%%uFFFD𐀀-󯿿]+$'), string.matchRegEx('1a', '^[:A-Z_a-z]')
# substring search without wrapper, anchors and flags
RETURN string.matchRegEx('xyz', 'y'), string.matchRegEx('xyz', '^y'), string.matchRegEx('x%%BS%%nyz', '^y'), string.matchRegEx('x%%BS%%nyz', '(?m)^y')
RETURN string.matchRegEx('X', '(?i)[a-z]'), string.matchRegEx('É', '(?i)é'), string.matchRegEx('ǅ', '(?i)ǆ'), string.matchRegEx('K', '(?i)k')
RETURN string.matchRegEx('ab', '(?s)(?s:.*)(?:b)(?s:.*)'), string.matchRegEx('', '(?:)'), string.matchRegEx('', 'a*')
RETURN size(string.matchRegEx('aaaa', 'a')), size(string.matchRegEx('aaaa', 'a')) > 0
# code point escapes in quantifier and class context
RETURN string.matchRegEx('AA', '%%BS%%%%BS%%u0041{2}'), string.matchRegEx('-', '[%%BS%%%%BS%%u002D]'), string.matchRegEx(']', '[%%BS%%%%BS%%u005D]'), string.matchRegEx('^', '[%%BS%%%%BS%%u005E]'), string.matchRegEx('%%BS%%%%BS%%', '[%%BS%%%%BS%%u005C]')
RETURN string.matchRegEx('a', '[%%BS%%%%BS%%u0061-%%BS%%%%BS%%u0060]')
PROBES
