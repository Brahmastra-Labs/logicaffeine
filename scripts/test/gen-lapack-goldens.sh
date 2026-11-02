#!/usr/bin/env bash
# Regenerate the netlib-reference golden vectors that lock LOGOS-BLAS against the
# canonical Fortran (BLAS/LAPACK). The reference source is fetched (curl, no git —
# CLAUDE.md rule 1) into the gitignored external/lapack/ study dir if absent, then a
# driver is linked against the *actual* reference routines, compiled with
# -O0 -fno-fast-math (pin the reference's own rounding), run, and its fixture copied
# into the committed test-data dir. The consuming test is blas_reference_vectors.rs.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LAPACK_VER="3.12.0"
LAPACK_DIR="${REPO_ROOT}/external/lapack/lapack-${LAPACK_VER}"
BLAS_SRC="${LAPACK_DIR}/BLAS/SRC"
ORACLE_DIR="${REPO_ROOT}/scripts/test/lapack-oracle"
DATA_DIR="${REPO_ROOT}/crates/logicaffeine_tests/tests/data/lapack"

command -v gfortran >/dev/null 2>&1 || { echo "gfortran not found — install it (Fortran reference oracle needs it)"; exit 1; }

if [[ ! -d "${LAPACK_DIR}" ]]; then
  echo "Fetching netlib LAPACK ${LAPACK_VER} reference (curl, no git)…"
  mkdir -p "${REPO_ROOT}/external/lapack"
  curl -fsSL "https://github.com/Reference-LAPACK/lapack/archive/refs/tags/v${LAPACK_VER}.tar.gz" \
    -o "/tmp/lapack-${LAPACK_VER}.tgz"
  tar -xzf "/tmp/lapack-${LAPACK_VER}.tgz" -C "${REPO_ROOT}/external/lapack/"
fi

mkdir -p "${DATA_DIR}"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

# gemm ← reference DGEMM (+ its LSAME/XERBLA dependencies)
echo "Building gemm oracle against reference DGEMM…"
gfortran -O0 -fno-fast-math \
  "${ORACLE_DIR}/driver_gemm.f90" \
  "${BLAS_SRC}/dgemm.f" "${BLAS_SRC}/lsame.f" "${BLAS_SRC}/xerbla.f" \
  -o "${WORK}/driver_gemm"
( cd "${WORK}" && ./driver_gemm )
cp "${WORK}/gemm_6.txt" "${DATA_DIR}/gemm_6.txt"

# Complex z* ← the reference ZGESV / ZPOTRF / ZGEEV, linked against the SYSTEM LAPACK
# (liblapack/libblas): those routines pull in dozens of dependencies, so linking the shared
# library is far cleaner than listing every .f. The comparison in complex_reference_vectors.rs is
# tolerance-based (the external LOGOS-vs-Fortran accumulation-order boundary), so the system build's
# optimization level is immaterial.
echo "Building complex z* oracle against system LAPACK…"
# Prefer the linker's own -l resolution (needs the -dev symlinks); otherwise fall back to the
# versioned runtime .so.3 by absolute path (this box ships liblapack.so.3 without the -dev symlink).
LAPACK_LINK="-llapack -lblas"
if ! echo 'end' | gfortran -x f95 - -llapack -lblas -o /dev/null >/dev/null 2>&1; then
  LAPACK_SO="$(ldconfig -p | grep -oE '/[^ ]*liblapack\.so\.3' | head -1)"
  BLAS_SO="$(ldconfig -p | grep -oE '/[^ ]*libblas\.so\.3' | head -1)"
  [[ -n "${LAPACK_SO}" && -n "${BLAS_SO}" ]] || { echo "no liblapack/libblas found (install liblapack3/libblas3 or the -dev packages)"; exit 1; }
  LAPACK_LINK="${LAPACK_SO} ${BLAS_SO}"
fi
gfortran -O0 -fno-fast-math \
  "${ORACLE_DIR}/driver_zlapack.f90" \
  ${LAPACK_LINK} \
  -o "${WORK}/driver_zlapack"
( cd "${WORK}" && ./driver_zlapack )
cp "${WORK}/zgesv.txt"  "${DATA_DIR}/zgesv.txt"
cp "${WORK}/zpotrf.txt" "${DATA_DIR}/zpotrf.txt"
cp "${WORK}/zgeev.txt"  "${DATA_DIR}/zgeev.txt"

echo "Wrote goldens to ${DATA_DIR}:"
ls -1 "${DATA_DIR}"
