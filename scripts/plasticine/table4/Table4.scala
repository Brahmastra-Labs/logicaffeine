// The Spatial PLDI'18 Table 4 credibility gate: Spatial's own regression apps (pir/regression/), unmodified, at
// Table 2's data sizes on Plasticine as Table 4 configures it — a 16 x 8 array of 64 PCUs and 64 PMUs. Table 4
// does not publish the parallelization it used, so the search over it is pre-registered here, before any run:
//
//   GEMM      A, B: 1024 x 1024; 64-element tiles, the 16-lane reduction; row and column unrolling in {1, 2, 4},
//             contraction unrolling in {1, 2}
//   BS        960,000 options in 256-option tiles; outer parallelization in {1, 2, 4, 8}; PIR's bfs split, as
//             the regression's own BlackScholes_0 runs
//   GDA       1024 rows, 96 dimensions, 128-row tiles; the two inner parallelizations in {1, 2, 4}
//   KMeans    200 iterations over 320 points of 32 dimensions in 64-point tiles, 32 centroids (Table 2 does not
//             state K, and the regression app's own gold accumulates a point's first K coordinates, so it holds only
//             at K = D); the three inner parallelizations in {1, 2}
//   TQ6       6,400,000 records in 64-record tiles; outer parallelization in {1, 2, 4, 8}
//
// PIR maps each point onto the array; the points closest to Table 4's utilization are simulated by Tungsten.
// scripts/plasticine/table4.sh copies this file into the toolchain's pirTest sources.
import spatial.dsl._

// The chain's `-b PIR` flag selects every backend of PIR's family, the simulating one too, so table4.sh names the
// one it wants in TABLE4_BACKEND: PIR maps onto the array, Tst simulates.
trait Table4Run extends SpatialTest {
  override def backends = sys.env.get("TABLE4_BACKEND") match {
    case Some("Tst") => Seq(Tst(row = 16, col = 8))
    case _           => Seq(PIR(row = 16, col = 8))
  }
}

abstract class Table4GEMM(li: scala.Int, lj: scala.Int, lkk: scala.Int)
    extends GEMM(dim = 1024, ts = 64, its = 64, ip = 16, loop_i = li, loop_j = lj, loop_kk = lkk) with Table4Run

abstract class Table4BS(op: scala.Int) extends BlackScholes(N = 960000, ts = 256, op = op, ip = 16) with Table4Run {
  override def pirArgs = super.pirArgs + " --split-algo=bfs"
}

abstract class Table4GDA(mp1: scala.Int, mp2: scala.Int)
    extends GDA(C = 96, R = 1024, ts = 128, op = 1, mp1 = mp1, mp2 = mp2, ip = 16) with Table4Run

abstract class Table4KMeans(mp1: scala.Int, mp2: scala.Int, mp3: scala.Int)
    extends Kmeans(K = 32, D = 32, ts = 64, ip = 16, op = 1, mp1 = mp1, mp2 = mp2, mp3 = mp3) with Table4Run {
  override def runtimeArgs = "200 320"
}

abstract class Table4TQ6(op: scala.Int) extends TPCHQ6(N = 6400000, ts = 64, op = op, ip = 16) with Table4Run

class Table4_GEMM_i1_j1_kk1 extends Table4GEMM(1, 1, 1)
class Table4_GEMM_i1_j2_kk1 extends Table4GEMM(1, 2, 1)
class Table4_GEMM_i1_j4_kk1 extends Table4GEMM(1, 4, 1)
class Table4_GEMM_i2_j1_kk1 extends Table4GEMM(2, 1, 1)
class Table4_GEMM_i2_j2_kk1 extends Table4GEMM(2, 2, 1)
class Table4_GEMM_i2_j4_kk1 extends Table4GEMM(2, 4, 1)
class Table4_GEMM_i4_j1_kk1 extends Table4GEMM(4, 1, 1)
class Table4_GEMM_i4_j2_kk1 extends Table4GEMM(4, 2, 1)
class Table4_GEMM_i4_j4_kk1 extends Table4GEMM(4, 4, 1)
class Table4_GEMM_i1_j1_kk2 extends Table4GEMM(1, 1, 2)
class Table4_GEMM_i1_j2_kk2 extends Table4GEMM(1, 2, 2)
class Table4_GEMM_i1_j4_kk2 extends Table4GEMM(1, 4, 2)
class Table4_GEMM_i2_j1_kk2 extends Table4GEMM(2, 1, 2)
class Table4_GEMM_i2_j2_kk2 extends Table4GEMM(2, 2, 2)
class Table4_GEMM_i2_j4_kk2 extends Table4GEMM(2, 4, 2)
class Table4_GEMM_i4_j1_kk2 extends Table4GEMM(4, 1, 2)
class Table4_GEMM_i4_j2_kk2 extends Table4GEMM(4, 2, 2)
class Table4_GEMM_i4_j4_kk2 extends Table4GEMM(4, 4, 2)
class Table4_BS_op1 extends Table4BS(1)
class Table4_BS_op2 extends Table4BS(2)
class Table4_BS_op4 extends Table4BS(4)
class Table4_BS_op8 extends Table4BS(8)
class Table4_GDA_mp1_mp1 extends Table4GDA(1, 1)
class Table4_GDA_mp1_mp2 extends Table4GDA(1, 2)
class Table4_GDA_mp1_mp4 extends Table4GDA(1, 4)
class Table4_GDA_mp2_mp1 extends Table4GDA(2, 1)
class Table4_GDA_mp2_mp2 extends Table4GDA(2, 2)
class Table4_GDA_mp2_mp4 extends Table4GDA(2, 4)
class Table4_GDA_mp4_mp1 extends Table4GDA(4, 1)
class Table4_GDA_mp4_mp2 extends Table4GDA(4, 2)
class Table4_GDA_mp4_mp4 extends Table4GDA(4, 4)
class Table4_KMeans_mp1_mp1_mp1 extends Table4KMeans(1, 1, 1)
class Table4_KMeans_mp1_mp1_mp2 extends Table4KMeans(1, 1, 2)
class Table4_KMeans_mp1_mp2_mp1 extends Table4KMeans(1, 2, 1)
class Table4_KMeans_mp1_mp2_mp2 extends Table4KMeans(1, 2, 2)
class Table4_KMeans_mp2_mp1_mp1 extends Table4KMeans(2, 1, 1)
class Table4_KMeans_mp2_mp1_mp2 extends Table4KMeans(2, 1, 2)
class Table4_KMeans_mp2_mp2_mp1 extends Table4KMeans(2, 2, 1)
class Table4_KMeans_mp2_mp2_mp2 extends Table4KMeans(2, 2, 2)
class Table4_TQ6_op1 extends Table4TQ6(1)
class Table4_TQ6_op2 extends Table4TQ6(2)
class Table4_TQ6_op4 extends Table4TQ6(4)
class Table4_TQ6_op8 extends Table4TQ6(8)
