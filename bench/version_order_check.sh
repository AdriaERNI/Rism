#!/usr/bin/env bash
# Assert that packaged versions sort in the right upgrade order for deb,
# rpm, and archlinux. Two independent legs:
#
#   1. Artifact leg (each manager checked only if its package is in DIST):
#      reads the version out of the packages that were just built (dpkg-deb
#      -f, rpm -qp, .PKGINFO) and compares it against the previous stable
#      and, for a pre-release tag, against the future final. A final tag's
#      artifacts must carry the plain version.
#   2. Synthetic leg: the same comparison set for both pre-release spelling
#      styles this repo tags (dotted "pr.25" and plain "beta1"), derived
#      exactly the way the linux-packages step in ci.yml derives it.
#
# Comparators are the package managers' own code: dpkg --compare-versions,
# rpm's internal vercmp via `rpm --eval '%{lua: print(rpm.vercmp(..))}'`
# (host rpm, else a fedora container — rpmdevtools is not assumed), and
# pacman vercmp (host pacman, else an archlinux container). A missing
# comparator fails the run loudly.
#
# Env:
#   RISM_VER   version with the leading "v" stripped from the tag
#              (e.g. 0.2.3-pr.25 or 0.2.3). Required.
#   RISM_PREV  previous stable version for the "greater than" leg.
#              Default 0.2.2.
#   DIST       directory holding the built packages. Default ./dist.
#   RPM_IMAGE / ARCH_IMAGE  container references (default fedora:41,
#              archlinux).
set -uo pipefail

VER="${RISM_VER:?RISM_VER is required (tag minus leading v)}"
VER="${VER#v}"
PREV="${RISM_PREV:-0.2.2}"
PREV="${PREV#v}"
DIST="${DIST:-dist}"
RPM_IMAGE="${RPM_IMAGE:-fedora:41}"
ARCH_IMAGE="${ARCH_IMAGE:-archlinux}"
fails=0

ok()   { printf 'ok:   %s\n' "$1"; }
bad()  { printf 'FAIL: %s\n' "$1"; fails=$((fails + 1)); }

have() { command -v "$1" >/dev/null 2>&1; }

# Every comparator returns a vercmp-style verdict on stdout:
#   <0 first is older, 0 equal, >0 first is newer.
dpkg_lt() { if dpkg --compare-versions "$1" lt "$2"; then echo -1; else echo 1; fi; }

rpm_cmp() {
  if have rpm; then
    rpm --eval "%{lua: print(rpm.vercmp(\"$1\", \"$2\"))}"
  elif have docker; then
    docker run --rm "$RPM_IMAGE" rpm --eval \
      "%{lua: print(rpm.vercmp(\"$1\", \"$2\"))}"
  else
    echo "rpm comparator unavailable (need rpm or docker)" >&2; echo ERR
  fi
}

arch_cmp() { # pass full pkgver-pkgrel forms so pkgrel participates
  if have vercmp && have pacman; then
    vercmp "$1" "$2"
  elif have docker; then
    docker run --rm "$ARCH_IMAGE" vercmp "$1" "$2"
  else
    echo "vercmp comparator unavailable (need pacman or docker)" >&2; echo ERR
  fi
}

# check_lt <desc> <comparator-fn> <newer> <older> : pass when vercmp says
# the first argument sorts strictly below the second.
check_lt() {
  local desc="$1" fn="$2" a="$3" b="$4" out verdict
  out=$("$fn" "$a" "$b") || { bad "$desc: comparator error"; return; }
  verdict=$(printf '%s\n' "$out" | tr -s '[:space:]' '\n' | tail -n1)
  case "$verdict" in
    -[0-9]*) ok "$desc ($a < $b)" ;;
    ERR*)    bad "$desc: $out" ;;
    *)       bad "$desc — got '$verdict' ($out)" ;;
  esac
}

# gt_prev runs the "sorts above the previous stable" leg only when a
# previous stable actually exists (a pre-release of the current stable has
# nothing below it to block, and comparing 0.2.2-beta1's packages against an
# already-shipped 0.2.2 would assert the wrong order).
gt_prev() { # gt_prev <desc> <fn> <prev-form> <pkg-form>
  if dpkg --compare-versions "$PREV" lt "$BASE"; then
    check_lt "$1" "$2" "$3" "$4"
  else
    ok "$1 (skipped: no earlier stable than $BASE)"
  fi
}

# --- version derivation (MUST mirror .github/workflows/ci.yml) --------------
if [[ "$VER" == *-* ]]; then
  BASE=${VER%%-*}; PRSUF=${VER#*-}; PRSUF=${PRSUF//-/.}
  PKGVER="$BASE~$PRSUF"        # deb + rpm: tilde sorts before the final
  ARCHVER="$BASE$PRSUF"        # arch: no tilde (pacman ranks it AFTER the
                               # end of a segment, and makepkg forbids it
                               # in pkgver); letters sort before the end.
else
  BASE="$VER"; PKGVER="$VER"; ARCHVER="$VER"
fi

# --- artifact leg -----------------------------------------------------------
shopt -s nullglob
debs=("$DIST"/*.deb); rpms=("$DIST"/*.rpm); pkgs=("$DIST"/*.pkg.tar.zst)

if [ ${#debs[@]} -gt 0 ]; then
  have dpkg-deb || { bad "deb artifacts present but dpkg-deb missing"; }
  av=$(dpkg-deb -f "${debs[0]}" Version)      # "<upstream>-<release>"
  up=${av%-*}
  gt_prev "artifact deb > prev stable" dpkg_lt "$PREV-1" "$av"
  if [[ "$VER" == *-* ]]; then
    check_lt "artifact deb < final" dpkg_lt "$av" "$BASE-1"
  elif [ "$up" = "$BASE" ]; then
    ok "artifact deb upstream == $BASE"
  else
    bad "artifact deb upstream '$up' != tag version '$BASE'"
  fi
fi

if [ ${#rpms[@]} -gt 0 ]; then
  if have rpm; then
    av=$(rpm -qp --qf '%{VERSION}' "${rpms[0]}" 2>/dev/null)
  else
    av=$(docker run --rm -v "$PWD/$DIST":/d:ro "$RPM_IMAGE" \
         rpm -qp --qf '%{VERSION}' "/d/${rpms[0]##*/}")
  fi
  gt_prev "artifact rpm > prev stable" rpm_cmp "$PREV" "$av"
  if [[ "$VER" == *-* ]]; then
    check_lt "artifact rpm < final" rpm_cmp "$av" "$BASE"
  elif [ "$av" = "$BASE" ]; then
    ok "artifact rpm version == $BASE"
  else
    bad "artifact rpm version '$av' != tag version '$BASE'"
  fi
fi

if [ ${#pkgs[@]} -gt 0 ]; then
  av=$(tar --zstd -xOf "${pkgs[0]}" .PKGINFO | sed -n 's/^pkgver = \(.*\)-[0-9.]*$/\1/p')
  gt_prev "artifact arch > prev stable" arch_cmp "$PREV-1" "$av-1"
  if [[ "$VER" == *-* ]]; then
    check_lt "artifact arch < final" arch_cmp "$av-1" "$BASE-1"
  elif [ "$av" = "$BASE" ]; then
    ok "artifact arch pkgver == $BASE"
  else
    bad "artifact arch pkgver '$av' != tag version '$BASE'"
  fi
fi

# --- synthetic leg: both pre-release spellings, all three managers ----------
for suf in pr.25 beta1; do
  p="$BASE~$suf"; a="$BASE$suf"
  check_lt "deb $p vs final" dpkg_lt "$p" "$BASE"
  gt_prev "deb $p vs prev" dpkg_lt "$PREV" "$p"
  check_lt "rpm $p vs final" rpm_cmp "$p" "$BASE"
  gt_prev "rpm $p vs prev" rpm_cmp "$PREV" "$p"
  check_lt "arch $a vs final" arch_cmp "$a-1" "$BASE-1"
  gt_prev "arch $a vs prev" arch_cmp "$PREV-1" "$a-1"
done
check_lt "deb series"  dpkg_lt "$BASE~pr.25" "$BASE~pr.26"
check_lt "rpm series"  rpm_cmp "$BASE~pr.25" "$BASE~pr.26"
check_lt "arch series" arch_cmp "$BASE""pr.25-1" "$BASE""pr.26-1"

echo
if [ "$fails" -gt 0 ]; then
  echo "version_order_check: $fails assertion(s) FAILED" >&2
  exit 1
fi
echo "version_order_check: all assertions passed"