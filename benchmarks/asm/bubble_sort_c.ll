; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/bubble_sort/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/bubble_sort/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@__stderrp = external local_unnamed_addr global ptr, align 8
@.str = private unnamed_addr constant [24 x i8] c"Usage: bubble_sort <n>\0A\00", align 1
@.str.1 = private unnamed_addr constant [4 x i8] c"%d\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %4, label %7

4:                                                ; preds = %2
  %5 = load ptr, ptr @__stderrp, align 8, !tbaa !6
  %6 = tail call i64 @fwrite(ptr nonnull @.str, i64 23, i64 1, ptr %5)
  br label %58

7:                                                ; preds = %2
  %8 = getelementptr inbounds ptr, ptr %1, i64 1
  %9 = load ptr, ptr %8, align 8, !tbaa !6
  %10 = tail call i32 @atoi(ptr nocapture noundef %9)
  %11 = sext i32 %10 to i64
  %12 = shl nsw i64 %11, 2
  %13 = tail call ptr @malloc(i64 noundef %12) #6
  %14 = icmp eq ptr %13, null
  br i1 %14, label %58, label %15

15:                                               ; preds = %7
  %16 = icmp sgt i32 %10, 0
  br i1 %16, label %17, label %39

17:                                               ; preds = %15
  %18 = zext i32 %10 to i64
  br label %22

19:                                               ; preds = %22
  %20 = add i32 %10, -1
  %21 = icmp sgt i32 %10, 1
  br i1 %21, label %32, label %39

22:                                               ; preds = %17, %22
  %23 = phi i64 [ 0, %17 ], [ %30, %22 ]
  %24 = phi i32 [ 42, %17 ], [ %26, %22 ]
  %25 = mul i32 %24, 1103515245
  %26 = add i32 %25, 12345
  %27 = lshr i32 %26, 16
  %28 = and i32 %27, 32767
  %29 = getelementptr inbounds i32, ptr %13, i64 %23
  store i32 %28, ptr %29, align 4, !tbaa !10
  %30 = add nuw nsw i64 %23, 1
  %31 = icmp eq i64 %30, %18
  br i1 %31, label %19, label %22, !llvm.loop !12

32:                                               ; preds = %19, %42
  %33 = phi i32 [ %44, %42 ], [ %20, %19 ]
  %34 = phi i32 [ %43, %42 ], [ 0, %19 ]
  %35 = icmp sgt i32 %20, %34
  br i1 %35, label %36, label %42

36:                                               ; preds = %32
  %37 = zext i32 %33 to i64
  %38 = load i32, ptr %13, align 4, !tbaa !10
  br label %46

39:                                               ; preds = %42, %15, %19
  %40 = load i32, ptr %13, align 4, !tbaa !10
  %41 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str.1, i32 noundef %40)
  tail call void @free(ptr noundef %13)
  br label %58

42:                                               ; preds = %55, %32
  %43 = add nuw nsw i32 %34, 1
  %44 = add i32 %33, -1
  %45 = icmp eq i32 %43, %20
  br i1 %45, label %39, label %32, !llvm.loop !14

46:                                               ; preds = %36, %55
  %47 = phi i32 [ %38, %36 ], [ %56, %55 ]
  %48 = phi i64 [ 0, %36 ], [ %49, %55 ]
  %49 = add nuw nsw i64 %48, 1
  %50 = getelementptr inbounds i32, ptr %13, i64 %49
  %51 = load i32, ptr %50, align 4, !tbaa !10
  %52 = icmp sgt i32 %47, %51
  br i1 %52, label %53, label %55

53:                                               ; preds = %46
  %54 = getelementptr inbounds i32, ptr %13, i64 %48
  store i32 %51, ptr %54, align 4, !tbaa !10
  store i32 %47, ptr %50, align 4, !tbaa !10
  br label %55

55:                                               ; preds = %46, %53
  %56 = phi i32 [ %51, %46 ], [ %47, %53 ]
  %57 = icmp eq i64 %49, %37
  br i1 %57, label %42, label %46, !llvm.loop !15

58:                                               ; preds = %39, %7, %4
  %59 = phi i32 [ 1, %4 ], [ 0, %39 ], [ 1, %7 ]
  ret i32 %59
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i32 @atoi(ptr nocapture noundef) local_unnamed_addr #1

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #4

; Function Attrs: nofree nounwind
declare noundef i64 @fwrite(ptr nocapture noundef, i64 noundef, i64 noundef, ptr nocapture noundef) local_unnamed_addr #5

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { nofree nounwind }
attributes #6 = { allocsize(0) }

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
!10 = !{!11, !11, i64 0}
!11 = !{!"int", !8, i64 0}
!12 = distinct !{!12, !13}
!13 = !{!"llvm.loop.mustprogress"}
!14 = distinct !{!14, !13}
!15 = distinct !{!15, !13}
