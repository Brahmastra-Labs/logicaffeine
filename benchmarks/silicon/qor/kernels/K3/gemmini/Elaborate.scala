// Elaborate Gemmini's MeshWithDelays as the K3 candidate: an 8x8 mesh of 1x1 tiles of int8 inputs and weights, one
// dataflow, Gemmini's default tile latency (0), mesh output delay (1) and one shifter bank, results `outBits` wide.
//
//   runMain qor.Elaborate <ws|os> <outBits> <target dir>
package qor

import chisel3._
import gemmini._

/** The tag MeshWithDelays carries with each matmul: one bit, in the place of the ExecuteController's `mesh_tag`. */
class BenchTag extends Bundle with TagQueueTag {
  val v = UInt(1.W)
  override def make_this_garbage(dummy: Int = 0): Unit = { v := 0.U }
}

object Elaborate extends App {
  val Array(df, outBits, dir) = args
  val dataflow = df match {
    case "ws" => Dataflow.WS
    case "os" => Dataflow.OS
  }
  (new chisel3.stage.ChiselStage).emitVerilog(
    new MeshWithDelays(SInt(8.W), SInt(8.W), SInt(outBits.toInt.W), SInt(32.W), new BenchTag, dataflow,
      tree_reduction = false, tile_latency = 0, output_delay = 1, tileRows = 1, tileColumns = 1,
      meshRows = 8, meshColumns = 8, leftBanks = 1, upBanks = 1),
    Array("--target-dir", dir))
}
