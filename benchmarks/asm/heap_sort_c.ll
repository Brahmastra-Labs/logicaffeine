; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/heap_sort/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/heap_sort/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [13 x i8] c"%ld %ld %ld\0A\00", align 1

; Function Attrs: nofree norecurse nosync nounwind ssp memory(argmem: readwrite) uwtable(sync)
define void @sift_down(ptr nocapture noundef %0, i64 noundef %1, i64 noundef %2) local_unnamed_addr #0 {
  %4 = shl nsw i64 %1, 1
  %5 = or i64 %4, 1
  %6 = icmp sgt i64 %5, %2
  br i1 %6, label %37, label %7

7:                                                ; preds = %3
  %8 = getelementptr inbounds i64, ptr %0, i64 %1
  %9 = load i64, ptr %8, align 8, !tbaa !6
  br label %10

10:                                               ; preds = %7, %31
  %11 = phi i64 [ %35, %31 ], [ %5, %7 ]
  %12 = phi i64 [ %34, %31 ], [ %4, %7 ]
  %13 = phi i64 [ %29, %31 ], [ %1, %7 ]
  %14 = getelementptr inbounds i64, ptr %0, i64 %13
  %15 = getelementptr inbounds i64, ptr %0, i64 %11
  %16 = load i64, ptr %15, align 8, !tbaa !6
  %17 = icmp slt i64 %9, %16
  %18 = select i1 %17, i64 %11, i64 %13
  %19 = add i64 %12, 2
  %20 = icmp sgt i64 %19, %2
  br i1 %20, label %28, label %21

21:                                               ; preds = %10
  %22 = getelementptr inbounds i64, ptr %0, i64 %18
  %23 = load i64, ptr %22, align 8, !tbaa !6
  %24 = getelementptr inbounds i64, ptr %0, i64 %19
  %25 = load i64, ptr %24, align 8, !tbaa !6
  %26 = icmp slt i64 %23, %25
  %27 = select i1 %26, i64 %19, i64 %18
  br label %28

28:                                               ; preds = %21, %10
  %29 = phi i64 [ %18, %10 ], [ %27, %21 ]
  %30 = icmp eq i64 %29, %13
  br i1 %30, label %37, label %31

31:                                               ; preds = %28
  %32 = getelementptr inbounds i64, ptr %0, i64 %29
  %33 = load i64, ptr %32, align 8, !tbaa !6
  store i64 %33, ptr %14, align 8, !tbaa !6
  store i64 %9, ptr %32, align 8, !tbaa !6
  %34 = shl nsw i64 %29, 1
  %35 = or i64 %34, 1
  %36 = icmp sgt i64 %35, %2
  br i1 %36, label %37, label %10

37:                                               ; preds = %31, %28, %3
  ret void
}

; Function Attrs: nofree norecurse nosync nounwind ssp memory(argmem: readwrite) uwtable(sync)
define void @heap_sort(ptr nocapture noundef %0, i64 noundef %1) local_unnamed_addr #0 {
  %3 = icmp sgt i64 %1, 0
  br i1 %3, label %4, label %47

4:                                                ; preds = %2
  %5 = add nsw i64 %1, -2
  %6 = sdiv i64 %5, 2
  br label %9

7:                                                ; preds = %44
  %8 = icmp sgt i64 %1, 1
  br i1 %8, label %48, label %47

9:                                                ; preds = %4, %44
  %10 = phi i64 [ %45, %44 ], [ %6, %4 ]
  %11 = shl nuw nsw i64 %10, 1
  %12 = or i64 %11, 1
  %13 = icmp slt i64 %12, %1
  br i1 %13, label %14, label %44

14:                                               ; preds = %9
  %15 = getelementptr inbounds i64, ptr %0, i64 %10
  %16 = load i64, ptr %15, align 8, !tbaa !6
  br label %17

17:                                               ; preds = %38, %14
  %18 = phi i64 [ %42, %38 ], [ %12, %14 ]
  %19 = phi i64 [ %41, %38 ], [ %11, %14 ]
  %20 = phi i64 [ %36, %38 ], [ %10, %14 ]
  %21 = getelementptr inbounds i64, ptr %0, i64 %20
  %22 = getelementptr inbounds i64, ptr %0, i64 %18
  %23 = load i64, ptr %22, align 8, !tbaa !6
  %24 = icmp slt i64 %16, %23
  %25 = select i1 %24, i64 %18, i64 %20
  %26 = add i64 %19, 2
  %27 = icmp slt i64 %26, %1
  br i1 %27, label %28, label %35

28:                                               ; preds = %17
  %29 = getelementptr inbounds i64, ptr %0, i64 %25
  %30 = load i64, ptr %29, align 8, !tbaa !6
  %31 = getelementptr inbounds i64, ptr %0, i64 %26
  %32 = load i64, ptr %31, align 8, !tbaa !6
  %33 = icmp slt i64 %30, %32
  %34 = select i1 %33, i64 %26, i64 %25
  br label %35

35:                                               ; preds = %28, %17
  %36 = phi i64 [ %25, %17 ], [ %34, %28 ]
  %37 = icmp eq i64 %36, %20
  br i1 %37, label %44, label %38

38:                                               ; preds = %35
  %39 = getelementptr inbounds i64, ptr %0, i64 %36
  %40 = load i64, ptr %39, align 8, !tbaa !6
  store i64 %40, ptr %21, align 8, !tbaa !6
  store i64 %16, ptr %39, align 8, !tbaa !6
  %41 = shl nsw i64 %36, 1
  %42 = or i64 %41, 1
  %43 = icmp slt i64 %42, %1
  br i1 %43, label %17, label %44

44:                                               ; preds = %35, %38, %9
  %45 = add nsw i64 %10, -1
  %46 = icmp sgt i64 %10, 0
  br i1 %46, label %9, label %7, !llvm.loop !10

47:                                               ; preds = %48, %85, %2, %7
  ret void

48:                                               ; preds = %7, %85
  %49 = phi i64 [ %50, %85 ], [ %1, %7 ]
  %50 = add nsw i64 %49, -1
  %51 = load i64, ptr %0, align 8, !tbaa !6
  %52 = getelementptr inbounds i64, ptr %0, i64 %50
  %53 = load i64, ptr %52, align 8, !tbaa !6
  store i64 %53, ptr %0, align 8, !tbaa !6
  store i64 %51, ptr %52, align 8, !tbaa !6
  %54 = add nsw i64 %49, -2
  %55 = icmp eq i64 %54, 0
  br i1 %55, label %47, label %56

56:                                               ; preds = %48
  %57 = load i64, ptr %0, align 8, !tbaa !6
  br label %58

58:                                               ; preds = %79, %56
  %59 = phi i64 [ %83, %79 ], [ 1, %56 ]
  %60 = phi i64 [ %82, %79 ], [ 0, %56 ]
  %61 = phi i64 [ %77, %79 ], [ 0, %56 ]
  %62 = getelementptr inbounds i64, ptr %0, i64 %61
  %63 = getelementptr inbounds i64, ptr %0, i64 %59
  %64 = load i64, ptr %63, align 8, !tbaa !6
  %65 = icmp slt i64 %57, %64
  %66 = select i1 %65, i64 %59, i64 %61
  %67 = add i64 %60, 2
  %68 = icmp sgt i64 %67, %54
  br i1 %68, label %76, label %69

69:                                               ; preds = %58
  %70 = getelementptr inbounds i64, ptr %0, i64 %66
  %71 = load i64, ptr %70, align 8, !tbaa !6
  %72 = getelementptr inbounds i64, ptr %0, i64 %67
  %73 = load i64, ptr %72, align 8, !tbaa !6
  %74 = icmp slt i64 %71, %73
  %75 = select i1 %74, i64 %67, i64 %66
  br label %76

76:                                               ; preds = %69, %58
  %77 = phi i64 [ %66, %58 ], [ %75, %69 ]
  %78 = icmp eq i64 %77, %61
  br i1 %78, label %85, label %79

79:                                               ; preds = %76
  %80 = getelementptr inbounds i64, ptr %0, i64 %77
  %81 = load i64, ptr %80, align 8, !tbaa !6
  store i64 %81, ptr %62, align 8, !tbaa !6
  store i64 %57, ptr %80, align 8, !tbaa !6
  %82 = shl nsw i64 %77, 1
  %83 = or i64 %82, 1
  %84 = icmp sgt i64 %83, %54
  br i1 %84, label %85, label %58

85:                                               ; preds = %76, %79
  %86 = icmp sgt i64 %49, 2
  br i1 %86, label %48, label %47, !llvm.loop !12
}

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #1 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %40, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !13
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = shl i64 %7, 3
  %9 = tail call ptr @malloc(i64 noundef %8) #6
  %10 = icmp sgt i64 %7, 0
  br i1 %10, label %13, label %11

11:                                               ; preds = %4
  tail call void @heap_sort(ptr noundef %9, i64 noundef %7)
  br label %24

12:                                               ; preds = %13
  tail call void @heap_sort(ptr noundef nonnull %9, i64 noundef %7)
  br i1 %10, label %31, label %24

13:                                               ; preds = %4, %13
  %14 = phi i64 [ %22, %13 ], [ 0, %4 ]
  %15 = phi i64 [ %18, %13 ], [ 42, %4 ]
  %16 = mul nuw nsw i64 %15, 1103515245
  %17 = add nuw nsw i64 %16, 12345
  %18 = and i64 %17, 2147483647
  %19 = lshr i64 %17, 16
  %20 = and i64 %19, 32767
  %21 = getelementptr inbounds i64, ptr %9, i64 %14
  store i64 %20, ptr %21, align 8, !tbaa !6
  %22 = add nuw nsw i64 %14, 1
  %23 = icmp eq i64 %22, %7
  br i1 %23, label %12, label %13, !llvm.loop !15

24:                                               ; preds = %31, %11, %12
  %25 = phi i64 [ 0, %12 ], [ 0, %11 ], [ %37, %31 ]
  %26 = load i64, ptr %9, align 8, !tbaa !6
  %27 = add nsw i64 %7, -1
  %28 = getelementptr inbounds i64, ptr %9, i64 %27
  %29 = load i64, ptr %28, align 8, !tbaa !6
  %30 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %26, i64 noundef %29, i64 noundef %25)
  tail call void @free(ptr noundef %9)
  br label %40

31:                                               ; preds = %12, %31
  %32 = phi i64 [ %38, %31 ], [ 0, %12 ]
  %33 = phi i64 [ %37, %31 ], [ 0, %12 ]
  %34 = getelementptr inbounds i64, ptr %9, i64 %32
  %35 = load i64, ptr %34, align 8, !tbaa !6
  %36 = add nsw i64 %35, %33
  %37 = srem i64 %36, 1000000007
  %38 = add nuw nsw i64 %32, 1
  %39 = icmp eq i64 %38, %7
  br i1 %39, label %24, label %31, !llvm.loop !16

40:                                               ; preds = %2, %24
  %41 = phi i32 [ 0, %24 ], [ 1, %2 ]
  ret i32 %41
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #2

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #3

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #4

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #5

attributes #0 = { nofree norecurse nosync nounwind ssp memory(argmem: readwrite) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
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
!7 = !{!"long", !8, i64 0}
!8 = !{!"omnipotent char", !9, i64 0}
!9 = !{!"Simple C/C++ TBAA"}
!10 = distinct !{!10, !11}
!11 = !{!"llvm.loop.mustprogress"}
!12 = distinct !{!12, !11}
!13 = !{!14, !14, i64 0}
!14 = !{!"any pointer", !8, i64 0}
!15 = distinct !{!15, !11}
!16 = distinct !{!16, !11}
