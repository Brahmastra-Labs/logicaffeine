; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/collatz/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/collatz/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@__stderrp = external local_unnamed_addr global ptr, align 8
@.str = private unnamed_addr constant [20 x i8] c"Usage: collatz <n>\0A\00", align 1
@.str.1 = private unnamed_addr constant [5 x i8] c"%ld\0A\00", align 1

; Function Attrs: nofree nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %4, label %7

4:                                                ; preds = %2
  %5 = load ptr, ptr @__stderrp, align 8, !tbaa !6
  %6 = tail call i64 @fwrite(ptr nonnull @.str, i64 19, i64 1, ptr %5)
  br label %37

7:                                                ; preds = %2
  %8 = getelementptr inbounds ptr, ptr %1, i64 1
  %9 = load ptr, ptr %8, align 8, !tbaa !6
  %10 = tail call i64 @atol(ptr nocapture noundef %9)
  %11 = icmp slt i64 %10, 1
  br i1 %11, label %16, label %12

12:                                               ; preds = %7, %33
  %13 = phi i64 [ %35, %33 ], [ 1, %7 ]
  %14 = phi i64 [ %34, %33 ], [ 0, %7 ]
  %15 = icmp eq i64 %13, 1
  br i1 %15, label %33, label %19

16:                                               ; preds = %33, %7
  %17 = phi i64 [ 0, %7 ], [ %34, %33 ]
  %18 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str.1, i64 noundef %17)
  br label %37

19:                                               ; preds = %12, %29
  %20 = phi i64 [ %30, %29 ], [ %13, %12 ]
  %21 = phi i64 [ %31, %29 ], [ %14, %12 ]
  %22 = and i64 %20, 1
  %23 = icmp eq i64 %22, 0
  br i1 %23, label %24, label %26

24:                                               ; preds = %19
  %25 = sdiv i64 %20, 2
  br label %29

26:                                               ; preds = %19
  %27 = mul nsw i64 %20, 3
  %28 = add nsw i64 %27, 1
  br label %29

29:                                               ; preds = %26, %24
  %30 = phi i64 [ %25, %24 ], [ %28, %26 ]
  %31 = add nsw i64 %21, 1
  %32 = icmp eq i64 %30, 1
  br i1 %32, label %33, label %19, !llvm.loop !10

33:                                               ; preds = %29, %12
  %34 = phi i64 [ %14, %12 ], [ %31, %29 ]
  %35 = add nuw i64 %13, 1
  %36 = icmp eq i64 %13, %10
  br i1 %36, label %16, label %12, !llvm.loop !12

37:                                               ; preds = %16, %4
  %38 = phi i32 [ 1, %4 ], [ 0, %16 ]
  ret i32 %38
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #1

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i64 @fwrite(ptr nocapture noundef, i64 noundef, i64 noundef, ptr nocapture noundef) local_unnamed_addr #3

attributes #0 = { nofree nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind }

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
