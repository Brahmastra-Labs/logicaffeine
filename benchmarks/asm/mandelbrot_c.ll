; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/mandelbrot/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/mandelbrot/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [4 x i8] c"%d\0A\00", align 1

; Function Attrs: nofree nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %58, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !6
  %7 = tail call i32 @atoi(ptr nocapture noundef %6)
  %8 = icmp sgt i32 %7, 0
  br i1 %8, label %9, label %19

9:                                                ; preds = %4
  %10 = sitofp i32 %7 to double
  br label %11

11:                                               ; preds = %22, %9
  %12 = phi i32 [ 0, %9 ], [ %55, %22 ]
  %13 = phi i32 [ 0, %9 ], [ %23, %22 ]
  %14 = sitofp i32 %13 to double
  %15 = fmul double %14, 2.000000e+00
  %16 = fdiv double %15, %10
  %17 = fadd double %16, -1.000000e+00
  %18 = fmul double %17, %17
  br label %25

19:                                               ; preds = %22, %4
  %20 = phi i32 [ 0, %4 ], [ %55, %22 ]
  %21 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i32 noundef %20)
  br label %58

22:                                               ; preds = %52
  %23 = add nuw nsw i32 %13, 1
  %24 = icmp eq i32 %23, %7
  br i1 %24, label %19, label %11, !llvm.loop !10

25:                                               ; preds = %11, %52
  %26 = phi i32 [ %12, %11 ], [ %55, %52 ]
  %27 = phi i32 [ 0, %11 ], [ %56, %52 ]
  %28 = sitofp i32 %27 to double
  %29 = fmul double %28, 2.000000e+00
  %30 = fdiv double %29, %10
  %31 = fadd double %30, -1.500000e+00
  %32 = tail call double @llvm.fmuladd.f64(double %31, double %31, double %18)
  %33 = fcmp ule double %32, 4.000000e+00
  br i1 %33, label %34, label %52

34:                                               ; preds = %25, %40
  %35 = phi double [ %46, %40 ], [ %17, %25 ]
  %36 = phi double [ %44, %40 ], [ %31, %25 ]
  %37 = phi i32 [ %38, %40 ], [ 0, %25 ]
  %38 = add nuw nsw i32 %37, 1
  %39 = icmp eq i32 %38, 50
  br i1 %39, label %50, label %40, !llvm.loop !12

40:                                               ; preds = %34
  %41 = fneg double %35
  %42 = fmul double %35, %41
  %43 = tail call double @llvm.fmuladd.f64(double %36, double %36, double %42)
  %44 = fadd double %31, %43
  %45 = fmul double %36, 2.000000e+00
  %46 = tail call double @llvm.fmuladd.f64(double %45, double %35, double %17)
  %47 = fmul double %46, %46
  %48 = tail call double @llvm.fmuladd.f64(double %44, double %44, double %47)
  %49 = fcmp ule double %48, 4.000000e+00
  br i1 %49, label %34, label %50, !llvm.loop !12

50:                                               ; preds = %34, %40
  %51 = icmp ugt i32 %37, 48
  br label %52

52:                                               ; preds = %50, %25
  %53 = phi i1 [ false, %25 ], [ %51, %50 ]
  %54 = zext i1 %53 to i32
  %55 = add nsw i32 %26, %54
  %56 = add nuw nsw i32 %27, 1
  %57 = icmp eq i32 %56, %7
  br i1 %57, label %22, label %25, !llvm.loop !13

58:                                               ; preds = %2, %19
  %59 = phi i32 [ 0, %19 ], [ 1, %2 ]
  ret i32 %59
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i32 @atoi(ptr nocapture noundef) local_unnamed_addr #1

; Function Attrs: mustprogress nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.fmuladd.f64(double, double, double) #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

attributes #0 = { nofree nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }

!llvm.module.flags = !{!0, !1, !2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i32 2, !"SDK Version", [2 x i32] [i32 15, i32 2]}
!1 = !{i32 1, !"wchar_size", i32 4}
!2 = !{i32 8, !"PIC Level", i32 2}
!3 = !{i32 7, !"uwtable", i32 1}
!4 = !{i32 7, !"frame-pointer", i32 1}
!5 = !{!"Apple clang version 16.0.0 (clang-1600.0.26.6)"}
!6 = !{!7, !7, i64 0}
!7 = !{!"any pointer", !8, i64 0}
!8 = !{!"omnipotent char", !9, i64 0}
!9 = !{!"Simple C/C++ TBAA"}
!10 = distinct !{!10, !11}
!11 = !{!"llvm.loop.mustprogress"}
!12 = distinct !{!12, !11}
!13 = distinct !{!13, !11}
