// The elaboration of Gemmini's MeshWithDelays for the comparative-synthesis bench, at Gemmini's own Scala and
// chisel3 versions (its build.sbt). scripts/qor/build-qor-candidate.py copies the Gemmini sources the mesh needs
// and the berkeley-hardfloat they import into src/main/scala beside Elaborate.scala.
scalaVersion := "2.13.10"
libraryDependencies += "edu.berkeley.cs" %% "chisel3" % "3.6.0"
addCompilerPlugin("edu.berkeley.cs" % "chisel3-plugin" % "3.6.0" cross CrossVersion.full)
scalacOptions ++= Seq("-language:reflectiveCalls", "-deprecation", "-feature")
