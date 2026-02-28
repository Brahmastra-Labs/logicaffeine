; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/gcd/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/gcd/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@__stderrp = external local_unnamed_addr global ptr, align 8
@.str = private unnamed_addr constant [16 x i8] c"Usage: gcd <n>\0A\00", align 1
@.str.1 = private unnamed_addr constant [5 x i8] c"%ld\0A\00", align 1

; Function Attrs: nofree norecurse nosync nounwind ssp memory(none) uwtable(sync)
define i64 @gcd(i64 noundef %0, i64 noundef %1) local_unnamed_addr #0 {
  %3 = icmp sgt i64 %1, 0
  br i1 %3, label %4, label %9

4:                                                ; preds = %2, %4
  %5 = phi i64 [ %6, %4 ], [ %0, %2 ]
  %6 = phi i64 [ %7, %4 ], [ %1, %2 ]
  %7 = srem i64 %5, %6
  %8 = icmp sgt i64 %7, 0
  br i1 %8, label %4, label %9, !llvm.loop !6

9:                                                ; preds = %4, %2
  %10 = phi i64 [ %0, %2 ], [ %6, %4 ]
  ret i64 %10
}

; Function Attrs: nofree nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #1 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %4, label %7

4:                                                ; preds = %2
  %5 = load ptr, ptr @__stderrp, align 8, !tbaa !8
  %6 = tail call i64 @fwrite(ptr nonnull @.str, i64 15, i64 1, ptr %5)
  br label %33

7:                                                ; preds = %2
  %8 = getelementptr inbounds ptr, ptr %1, i64 1
  %9 = load ptr, ptr %8, align 8, !tbaa !8
  %10 = tail call i64 @atol(ptr nocapture noundef %9)
  %11 = icmp slt i64 %10, 1
  br i1 %11, label %15, label %12

12:                                               ; preds = %7, %21
  %13 = phi i64 [ %22, %21 ], [ 1, %7 ]
  %14 = phi i64 [ %30, %21 ], [ 0, %7 ]
  br label %18

15:                                               ; preds = %21, %7
  %16 = phi i64 [ 0, %7 ], [ %30, %21 ]
  %17 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str.1, i64 noundef %16)
  br label %33

18:                                               ; preds = %12, %29
  %19 = phi i64 [ %13, %12 ], [ %31, %29 ]
  %20 = phi i64 [ %14, %12 ], [ %30, %29 ]
  br label %24

21:                                               ; preds = %29
  %22 = add nuw i64 %13, 1
  %23 = icmp eq i64 %13, %10
  br i1 %23, label %15, label %12, !llvm.loop !12

24:                                               ; preds = %18, %24
  %25 = phi i64 [ %26, %24 ], [ %13, %18 ]
  %26 = phi i64 [ %27, %24 ], [ %19, %18 ]
  %27 = srem i64 %25, %26
  %28 = icmp sgt i64 %27, 0
  br i1 %28, label %24, label %29, !llvm.loop !6

29:                                               ; preds = %24
  %30 = add nsw i64 %26, %20
  %31 = add nuw nsw i64 %19, 1
  %32 = icmp slt i64 %19, %10
  br i1 %32, label %18, label %21, !llvm.loop !13

33:                                               ; preds = %15, %4
  %34 = phi i32 [ 1, %4 ], [ 0, %15 ]
  ret i32 %34
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

; Function Attrs: nofree nounwind
declare noundef i64 @fwrite(ptr nocapture noundef, i64 noundef, i64 noundef, ptr nocapture noundef) local_unnamed_addr #4

attributes #0 = { nofree norecurse nosync nounwind ssp memory(none) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { nofree nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { nofree nounwind }

!llvm.module.flags = !{!0, !1, !2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i32 2, !"SDK Version", [2 x i32] [i32 15, i32 2]}
!1 = !{i32 1, !"wchar_size", i32 4}
!2 = !{i32 8, !"PIC Level", i32 2}
!3 = !{i32 7, !"uwtable", i32 1}
!4 = !{i32 7, !"frame-pointer", i32 1}
!5 = !{!"Apple clang version 16.0.0 (clang-1600.0.26.6)"}
!6 = distinct !{!6, !7}
!7 = !{!"llvm.loop.mustprogress"}
!8 = !{!9, !9, i64 0}
!9 = !{!"any pointer", !10, i64 0}
!10 = !{!"omnipotent char", !11, i64 0}
!11 = !{!"Simple C/C++ TBAA"}
!12 = distinct !{!12, !7}
!13 = distinct !{!13, !7}
