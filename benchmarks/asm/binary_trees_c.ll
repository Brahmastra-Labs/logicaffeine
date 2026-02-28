; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/binary_trees/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/binary_trees/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [38 x i8] c"stretch tree of depth %d\09 check: %ld\0A\00", align 1
@.str.1 = private unnamed_addr constant [35 x i8] c"%d\09 trees of depth %d\09 check: %ld\0A\00", align 1
@.str.2 = private unnamed_addr constant [41 x i8] c"long lived tree of depth %d\09 check: %ld\0A\00", align 1

; Function Attrs: nofree nosync nounwind ssp memory(none) uwtable(sync)
define i64 @make_check(i32 noundef %0) local_unnamed_addr #0 {
  %2 = icmp eq i32 %0, 0
  br i1 %2, label %3, label %5

3:                                                ; preds = %1, %5
  %4 = phi i64 [ %9, %5 ], [ 1, %1 ]
  ret i64 %4

5:                                                ; preds = %1
  %6 = add nsw i32 %0, -1
  %7 = tail call i64 @make_check(i32 noundef %6)
  %8 = shl i64 %7, 1
  %9 = or i64 %8, 1
  br label %3
}

; Function Attrs: nofree nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #1 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %31, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !6
  %7 = tail call i32 @atoi(ptr nocapture noundef %6)
  %8 = tail call i32 @llvm.smax.i32(i32 %7, i32 6)
  %9 = add nuw nsw i32 %8, 1
  %10 = tail call i64 @make_check(i32 noundef %9)
  %11 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i32 noundef %9, i64 noundef %10)
  %12 = tail call i64 @make_check(i32 noundef %8)
  %13 = add nuw i32 %8, 4
  br label %16

14:                                               ; preds = %26
  %15 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str.2, i32 noundef %8, i64 noundef %12)
  br label %31

16:                                               ; preds = %4, %26
  %17 = phi i32 [ 4, %4 ], [ %29, %26 ]
  %18 = sub i32 %13, %17
  %19 = shl nuw i32 1, %18
  %20 = icmp eq i32 %18, 31
  br i1 %20, label %26, label %21

21:                                               ; preds = %16
  %22 = tail call i64 @make_check(i32 noundef %17)
  %23 = tail call i32 @llvm.smax.i32(i32 %19, i32 1)
  %24 = zext i32 %23 to i64
  %25 = mul i64 %22, %24
  br label %26

26:                                               ; preds = %21, %16
  %27 = phi i64 [ 0, %16 ], [ %25, %21 ]
  %28 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str.1, i32 noundef %19, i32 noundef %17, i64 noundef %27)
  %29 = add nuw nsw i32 %17, 2
  %30 = icmp ugt i32 %29, %8
  br i1 %30, label %14, label %16, !llvm.loop !10

31:                                               ; preds = %2, %14
  %32 = phi i32 [ 0, %14 ], [ 1, %2 ]
  ret i32 %32
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i32 @atoi(ptr nocapture noundef) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i32 @llvm.smax.i32(i32, i32) #4

attributes #0 = { nofree nosync nounwind ssp memory(none) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { nofree nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }

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
