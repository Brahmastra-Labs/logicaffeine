! Golden-vector generator for LOGOS complex LAPACK (`clapack.lg`), validated against the reference
! ZGESV / ZPOTRF / ZGEEV (system LAPACK). Deterministic complex inputs are computed here, the
! reference routine is applied, and BOTH inputs and outputs are written to per-routine fixtures — so
! the LOGOS-side test consumes identical inputs from one authority and can never drift from a
! re-derived formula. Complex values are emitted as two lines (real then imaginary), full f64
! precision, in logical ROW-MAJOR order (i outer, j inner); LOGOS stores matrices row-major
! flattened, so a fixture element maps onto `item ((i-1)*n + j) of ...`. LAPACK works column-major
! internally; the row-major emission is the only bridge needed.
program driver_zlapack
  implicit none
  call gen_zgesv()
  call gen_zpotrf()
  call gen_zgeev()
contains

  subroutine emitz(u, z)
    integer, intent(in) :: u
    complex*16, intent(in) :: z
    write(u,'(ES30.17E3)') dble(z)
    write(u,'(ES30.17E3)') aimag(z)
  end subroutine emitz

  ! ZGESV: solve A·x = b for a well-conditioned (diagonally dominant) complex system.
  subroutine gen_zgesv()
    integer, parameter :: n = 4
    complex*16 :: A(n,n), Asave(n,n), b(n), bsave(n)
    integer :: ipiv(n), info, i, j, u
    do i = 1, n
      do j = 1, n
        A(i,j) = dcmplx(dble(mod(i*3 + j*2, 7))/3.0d0 - 1.0d0, dble(mod(i*5 + j, 5))/4.0d0 - 0.5d0)
      end do
      A(i,i) = A(i,i) + dcmplx(dble(n)*2.0d0, 0.0d0)
      b(i) = dcmplx(dble(i), dble(n - i))
    end do
    Asave = A
    bsave = b
    call zgesv(n, 1, A, n, ipiv, b, n, info)
    open(newunit=u, file='zgesv.txt', status='replace', action='write')
    write(u,'(A)') '# zgesv reference (system LAPACK): n, then A (n*n), b (n), x (n) row-major, re+im'
    write(u,'(I0)') n
    do i = 1, n
      do j = 1, n
        call emitz(u, Asave(i,j))
      end do
    end do
    do i = 1, n
      call emitz(u, bsave(i))
    end do
    do i = 1, n
      call emitz(u, b(i))
    end do
    close(u)
  end subroutine gen_zgesv

  ! ZPOTRF: Cholesky of a Hermitian positive-definite A = L0·L0ᴴ (built from a fixed lower factor,
  ! so A is genuinely HPD). Emit A and the recovered lower L (strict upper zeroed to match LOGOS).
  subroutine gen_zpotrf()
    integer, parameter :: n = 3
    complex*16 :: L0(n,n), A(n,n), L(n,n)
    integer :: info, i, j, u
    L0 = (0.0d0, 0.0d0)
    L0(1,1) = dcmplx(2.0d0, 0.0d0)
    L0(2,1) = dcmplx(1.0d0, 1.0d0)
    L0(2,2) = dcmplx(2.0d0, 0.0d0)
    L0(3,1) = dcmplx(1.0d0, -1.0d0)
    L0(3,2) = dcmplx(1.0d0, 1.0d0)
    L0(3,3) = dcmplx(3.0d0, 0.0d0)
    A = matmul(L0, conjg(transpose(L0)))
    L = A
    call zpotrf('L', n, L, n, info)
    do i = 1, n
      do j = i + 1, n
        L(i,j) = (0.0d0, 0.0d0)
      end do
    end do
    open(newunit=u, file='zpotrf.txt', status='replace', action='write')
    write(u,'(A)') '# zpotrf reference (system LAPACK): n, then A (n*n), L (n*n) row-major, re+im'
    write(u,'(I0)') n
    do i = 1, n
      do j = 1, n
        call emitz(u, A(i,j))
      end do
    end do
    do i = 1, n
      do j = 1, n
        call emitz(u, L(i,j))
      end do
    end do
    close(u)
  end subroutine gen_zpotrf

  ! ZGEEV: eigenvalues of a general complex matrix (values only).
  subroutine gen_zgeev()
    integer, parameter :: n = 4
    integer, parameter :: lwork = 8*n
    complex*16 :: A(n,n), Asave(n,n), w(n), vl(1,1), vr(1,1), work(lwork)
    double precision :: rwork(2*n)
    integer :: info, i, j, u
    do i = 1, n
      do j = 1, n
        A(i,j) = dcmplx(dble(mod(i*2 + j*5, 9))/2.0d0 - 2.0d0, dble(mod(i*7 + j*3, 6))/3.0d0 - 1.0d0)
      end do
    end do
    Asave = A
    call zgeev('N', 'N', n, A, n, w, vl, 1, vr, 1, work, lwork, rwork, info)
    open(newunit=u, file='zgeev.txt', status='replace', action='write')
    write(u,'(A)') '# zgeev reference (system LAPACK): n, then A (n*n) row-major, then eigenvalues (n), re+im'
    write(u,'(I0)') n
    do i = 1, n
      do j = 1, n
        call emitz(u, Asave(i,j))
      end do
    end do
    do i = 1, n
      call emitz(u, w(i))
    end do
    close(u)
  end subroutine gen_zgeev

end program driver_zlapack
