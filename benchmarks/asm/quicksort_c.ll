; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/quicksort/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/quicksort/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [13 x i8] c"%ld %ld %ld\0A\00", align 1

; Function Attrs: mustprogress nofree norecurse nosync nounwind ssp willreturn memory(argmem: readwrite) uwtable(sync)
define void @swap(ptr nocapture noundef %0, ptr nocapture noundef %1) local_unnamed_addr #0 {
  %3 = load i64, ptr %0, align 8, !tbaa !6
  %4 = load i64, ptr %1, align 8, !tbaa !6
  store i64 %4, ptr %0, align 8, !tbaa !6
  store i64 %3, ptr %1, align 8, !tbaa !6
  ret void
}

; Function Attrs: nofree norecurse nosync nounwind ssp memory(argmem: readwrite) uwtable(sync)
define i64 @partition(ptr nocapture noundef %0, i64 noundef %1, i64 noundef %2) local_unnamed_addr #1 {
  %4 = getelementptr inbounds i64, ptr %0, i64 %2
  %5 = load i64, ptr %4, align 8, !tbaa !6
  %6 = icmp slt i64 %1, %2
  br i1 %6, label %14, label %9

7:                                                ; preds = %24
  %8 = load i64, ptr %4, align 8, !tbaa !6
  br label %9

9:                                                ; preds = %7, %3
  %10 = phi i64 [ %5, %3 ], [ %8, %7 ]
  %11 = phi i64 [ %1, %3 ], [ %25, %7 ]
  %12 = getelementptr inbounds i64, ptr %0, i64 %11
  %13 = load i64, ptr %12, align 8, !tbaa !6
  store i64 %10, ptr %12, align 8, !tbaa !6
  store i64 %13, ptr %4, align 8, !tbaa !6
  ret i64 %11

14:                                               ; preds = %3, %24
  %15 = phi i64 [ %26, %24 ], [ %1, %3 ]
  %16 = phi i64 [ %25, %24 ], [ %1, %3 ]
  %17 = getelementptr inbounds i64, ptr %0, i64 %15
  %18 = load i64, ptr %17, align 8, !tbaa !6
  %19 = icmp sgt i64 %18, %5
  br i1 %19, label %24, label %20

20:                                               ; preds = %14
  %21 = getelementptr inbounds i64, ptr %0, i64 %16
  %22 = load i64, ptr %21, align 8, !tbaa !6
  store i64 %18, ptr %21, align 8, !tbaa !6
  store i64 %22, ptr %17, align 8, !tbaa !6
  %23 = add nsw i64 %16, 1
  br label %24

24:                                               ; preds = %14, %20
  %25 = phi i64 [ %23, %20 ], [ %16, %14 ]
  %26 = add nsw i64 %15, 1
  %27 = icmp eq i64 %26, %2
  br i1 %27, label %7, label %14, !llvm.loop !10
}

; Function Attrs: nofree nosync nounwind ssp memory(argmem: readwrite) uwtable(sync)
define void @qs(ptr nocapture noundef %0, i64 noundef %1, i64 noundef %2) local_unnamed_addr #2 {
  %4 = icmp slt i64 %1, %2
  br i1 %4, label %5, label %31

5:                                                ; preds = %3
  %6 = getelementptr inbounds i64, ptr %0, i64 %2
  br label %7

7:                                                ; preds = %5, %24
  %8 = phi i64 [ %1, %5 ], [ %29, %24 ]
  %9 = load i64, ptr %6, align 8, !tbaa !6
  br label %10

10:                                               ; preds = %7, %20
  %11 = phi i64 [ %22, %20 ], [ %8, %7 ]
  %12 = phi i64 [ %21, %20 ], [ %8, %7 ]
  %13 = getelementptr inbounds i64, ptr %0, i64 %11
  %14 = load i64, ptr %13, align 8, !tbaa !6
  %15 = icmp sgt i64 %14, %9
  br i1 %15, label %20, label %16

16:                                               ; preds = %10
  %17 = getelementptr inbounds i64, ptr %0, i64 %12
  %18 = load i64, ptr %17, align 8, !tbaa !6
  store i64 %14, ptr %17, align 8, !tbaa !6
  store i64 %18, ptr %13, align 8, !tbaa !6
  %19 = add nsw i64 %12, 1
  br label %20

20:                                               ; preds = %16, %10
  %21 = phi i64 [ %19, %16 ], [ %12, %10 ]
  %22 = add nsw i64 %11, 1
  %23 = icmp eq i64 %22, %2
  br i1 %23, label %24, label %10, !llvm.loop !10

24:                                               ; preds = %20
  %25 = load i64, ptr %6, align 8, !tbaa !6
  %26 = getelementptr inbounds i64, ptr %0, i64 %21
  %27 = load i64, ptr %26, align 8, !tbaa !6
  store i64 %25, ptr %26, align 8, !tbaa !6
  store i64 %27, ptr %6, align 8, !tbaa !6
  %28 = add nsw i64 %21, -1
  tail call void @qs(ptr noundef nonnull %0, i64 noundef %8, i64 noundef %28)
  %29 = add nsw i64 %21, 1
  %30 = icmp slt i64 %29, %2
  br i1 %30, label %7, label %31

31:                                               ; preds = %24, %3
  ret void
}

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #3 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %42, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !12
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = shl i64 %7, 3
  %9 = tail call ptr @malloc(i64 noundef %8) #8
  %10 = icmp sgt i64 %7, 0
  br i1 %10, label %15, label %11

11:                                               ; preds = %4
  %12 = add nsw i64 %7, -1
  tail call void @qs(ptr noundef %9, i64 noundef 0, i64 noundef %12)
  br label %26

13:                                               ; preds = %15
  %14 = add nsw i64 %7, -1
  tail call void @qs(ptr noundef nonnull %9, i64 noundef 0, i64 noundef %14)
  br i1 %10, label %33, label %26

15:                                               ; preds = %4, %15
  %16 = phi i64 [ %24, %15 ], [ 0, %4 ]
  %17 = phi i64 [ %20, %15 ], [ 42, %4 ]
  %18 = mul nuw nsw i64 %17, 1103515245
  %19 = add nuw nsw i64 %18, 12345
  %20 = and i64 %19, 2147483647
  %21 = lshr i64 %19, 16
  %22 = and i64 %21, 32767
  %23 = getelementptr inbounds i64, ptr %9, i64 %16
  store i64 %22, ptr %23, align 8, !tbaa !6
  %24 = add nuw nsw i64 %16, 1
  %25 = icmp eq i64 %24, %7
  br i1 %25, label %13, label %15, !llvm.loop !14

26:                                               ; preds = %33, %11, %13
  %27 = phi i64 [ %14, %13 ], [ %12, %11 ], [ %14, %33 ]
  %28 = phi i64 [ 0, %13 ], [ 0, %11 ], [ %39, %33 ]
  %29 = load i64, ptr %9, align 8, !tbaa !6
  %30 = getelementptr inbounds i64, ptr %9, i64 %27
  %31 = load i64, ptr %30, align 8, !tbaa !6
  %32 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %29, i64 noundef %31, i64 noundef %28)
  tail call void @free(ptr noundef %9)
  br label %42

33:                                               ; preds = %13, %33
  %34 = phi i64 [ %40, %33 ], [ 0, %13 ]
  %35 = phi i64 [ %39, %33 ], [ 0, %13 ]
  %36 = getelementptr inbounds i64, ptr %9, i64 %34
  %37 = load i64, ptr %36, align 8, !tbaa !6
  %38 = add nsw i64 %37, %35
  %39 = srem i64 %38, 1000000007
  %40 = add nuw nsw i64 %34, 1
  %41 = icmp eq i64 %40, %7
  br i1 %41, label %26, label %33, !llvm.loop !15

42:                                               ; preds = %2, %26
  %43 = phi i32 [ 0, %26 ], [ 1, %2 ]
  ret i32 %43
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #4

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #5

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #6

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #7

attributes #0 = { mustprogress nofree norecurse nosync nounwind ssp willreturn memory(argmem: readwrite) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { nofree norecurse nosync nounwind ssp memory(argmem: readwrite) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { nofree nosync nounwind ssp memory(argmem: readwrite) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #6 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #7 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #8 = { allocsize(0) }

!llvm.module.flags = !{!0, !1, !2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i32 2, !"SDK Version", [2 x i32] [i32 15, i32 2]}
!1 = !{i32 1, !"wchar_size", i32 4}
!2 = !{i32 8, !"PIC Level", i32 2}
!3 = !{i32 7, !"uwtable", i32 1}
!4 = !{i32 7, !"frame-pointer", i32 1}
!5 = !{!"Apple clang version 16.0.0 (clang-1600.0.26.6)"}
!6 = !{!7, !7, i64 0}
!7 = !{!"long", !8, i64 0}
!8 = !{!"omnipotent char", !9, i64 0}
!9 = !{!"Simple C/C++ TBAA"}
!10 = distinct !{!10, !11}
!11 = !{!"llvm.loop.mustprogress"}
!12 = !{!13, !13, i64 0}
!13 = !{!"any pointer", !8, i64 0}
!14 = distinct !{!14, !11}
!15 = distinct !{!15, !11}
