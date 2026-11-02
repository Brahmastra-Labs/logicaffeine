! Golden-vector generator for LOGOS-BLAS `gemm`, validated against the netlib
! reference DGEMM (BLAS 3.12.0). Deterministic fractional inputs are computed here,
! the reference product is taken, and BOTH inputs and output are written to the
! fixture — so the LOGOS-side test consumes identical inputs from one authority and
! can never drift from a re-derived formula. Values are emitted in logical ROW-MAJOR
! order (i outer, j inner); LOGOS stores matrices row-major flattened, so the fixture
! maps element-for-element onto `item ((i-1)*n + j) of ...`.
!
! Reference DGEMM works column-major internally; the row-major emission order is the
! only bridge needed. Full f64 precision (18 significant digits) round-trips exactly.
program driver_gemm
  implicit none
  integer, parameter :: m = 6, n = 6, k = 6
  double precision :: A(m,k), B(k,n), C(m,n)
  double precision :: alpha, beta
  integer :: i, j, l, u

  do l = 1, k
    do i = 1, m
      A(i,l) = 1.0d0 / dble(i + l)                     ! Hilbert-ish, well-conditioned fractions
    end do
  end do
  do j = 1, n
    do l = 1, k
      B(l,j) = dble(mod(l*3 + j*7, 11)) / 4.0d0 - 1.0d0 ! signed spread in [-1, 1.5]
    end do
  end do

  alpha = 1.0d0
  beta  = 0.0d0
  C     = 0.0d0

  call dgemm('N', 'N', m, n, k, alpha, A, m, B, k, beta, C, m)

  open(newunit=u, file='gemm_6.txt', status='replace', action='write')
  write(u,'(A)') '# gemm reference (netlib DGEMM v3.12.0): m n k alpha beta'
  write(u,'(I0,1X,I0,1X,I0,1X,A,1X,A)') m, n, k, '1.0', '0.0'
  write(u,'(A)') '# A (m*k, row-major)'
  do i = 1, m
    do l = 1, k
      write(u,'(ES30.17E3)') A(i,l)
    end do
  end do
  write(u,'(A)') '# B (k*n, row-major)'
  do l = 1, k
    do j = 1, n
      write(u,'(ES30.17E3)') B(l,j)
    end do
  end do
  write(u,'(A)') '# C = A*B (m*n, row-major)'
  do i = 1, m
    do j = 1, n
      write(u,'(ES30.17E3)') C(i,j)
    end do
  end do
  close(u)
end program driver_gemm
