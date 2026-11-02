import spatial.dsl._

/** K3 gemm8 written in Spatial to the bench's contract: eight rows of eight int8 activations against one 8x8 int8
  * weight tile, int32 sums, o(i)(c) = sum over r of a(i)(r) * w(r)(c). Row i of the activations and row r of the
  * weights each arrive packed in one 64-bit ArgIn (lane k in bits 8k+7..8k); the 64 sums leave through ArgOuts, row
  * major. The tile is unpacked into register files, then a pipelined loop takes one row a cycle through 64
  * multiply-adds (the eight sums of eight products of a row), and the sums are written out. */
@spatial object QorGemm8 extends SpatialApp {
  type A = I8
  type O = I32

  def main(args: Array[String]): Unit = {
    val aIn = List.tabulate(8) { _ => ArgIn[I64] }
    val wIn = List.tabulate(8) { _ => ArgIn[I64] }
    val oOut = List.tabulate(64) { _ => ArgOut[O] }
    List.tabulate(8) { i => setArg(aIn(i), args(i).to[I64]) }
    List.tabulate(8) { r => setArg(wIn(r), args(8 + r).to[I64]) }

    Accel {
      val a = RegFile[A](8, 8)
      val w = RegFile[A](8, 8)
      val o = RegFile[O](8, 8)
      Pipe {
        List.tabulate(8) { i =>
          List.tabulate(8) { k =>
            a(i, k) = aIn(i).value.bits(8 * k + 7 :: 8 * k).as[A]
            w(i, k) = wIn(i).value.bits(8 * k + 7 :: 8 * k).as[A]
          }
        }
      }
      Foreach(8 by 1) { i =>
        List.tabulate(8) { c =>
          o(i, c) = List.tabulate(8) { r => a(i, r).to[O] * w(r, c).to[O] }.reduceTree { _ + _ }
        }
      }
      Pipe {
        List.tabulate(8) { i => List.tabulate(8) { c => oOut(8 * i + c) := o(i, c) } }
      }
    }

    List.tabulate(64) { k => println("o" + k + " = " + getArg(oOut(k))) }
    println("gemm8 done")
  }
}
