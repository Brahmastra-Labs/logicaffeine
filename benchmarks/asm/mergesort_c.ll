; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/mergesort/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/mergesort/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [13 x i8] c"%ld %ld %ld\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define void @merge_sort(ptr nocapture noundef %0, i64 noundef %1) local_unnamed_addr #0 {
  %3 = icmp slt i64 %1, 2
  br i1 %3, label %58, label %4

4:                                                ; preds = %2
  %5 = lshr i64 %1, 1
  %6 = shl i64 %5, 3
  %7 = tail call ptr @malloc(i64 noundef %6) #7
  %8 = sub nsw i64 %1, %5
  %9 = shl i64 %8, 3
  %10 = tail call ptr @malloc(i64 noundef %9) #7
  tail call void @llvm.memcpy.p0.p0.i64(ptr noundef align 1 %7, ptr noundef align 1 %0, i64 noundef %6, i1 noundef false) #8
  %11 = getelementptr inbounds i64, ptr %0, i64 %5
  tail call void @llvm.memcpy.p0.p0.i64(ptr noundef align 1 %10, ptr noundef align 1 %11, i64 noundef %9, i1 noundef false) #8
  tail call void @merge_sort(ptr noundef %7, i64 noundef %5)
  tail call void @merge_sort(ptr noundef %10, i64 noundef %8)
  %12 = icmp sgt i64 %8, 0
  br i1 %12, label %26, label %13

13:                                               ; preds = %26, %4
  %14 = phi i64 [ 0, %4 ], [ %37, %26 ]
  %15 = phi i64 [ 0, %4 ], [ %40, %26 ]
  %16 = phi i64 [ 0, %4 ], [ %42, %26 ]
  %17 = icmp slt i64 %14, %5
  br i1 %17, label %18, label %46

18:                                               ; preds = %13
  %19 = shl i64 %16, 3
  %20 = getelementptr i8, ptr %0, i64 %19
  %21 = shl i64 %14, 3
  %22 = getelementptr i8, ptr %7, i64 %21
  %23 = sub i64 %6, %21
  tail call void @llvm.memcpy.p0.p0.i64(ptr align 8 %20, ptr align 8 %22, i64 %23, i1 false), !tbaa !6
  %24 = add i64 %16, %5
  %25 = sub i64 %24, %14
  br label %46

26:                                               ; preds = %4, %26
  %27 = phi i64 [ %42, %26 ], [ 0, %4 ]
  %28 = phi i64 [ %40, %26 ], [ 0, %4 ]
  %29 = phi i64 [ %37, %26 ], [ 0, %4 ]
  %30 = getelementptr inbounds i64, ptr %7, i64 %29
  %31 = load i64, ptr %30, align 8, !tbaa !6
  %32 = getelementptr inbounds i64, ptr %10, i64 %28
  %33 = load i64, ptr %32, align 8, !tbaa !6
  %34 = icmp sle i64 %31, %33
  %35 = tail call i64 @llvm.smin.i64(i64 %31, i64 %33)
  %36 = zext i1 %34 to i64
  %37 = add nuw nsw i64 %29, %36
  %38 = xor i1 %34, true
  %39 = zext i1 %38 to i64
  %40 = add nuw nsw i64 %28, %39
  %41 = getelementptr inbounds i64, ptr %0, i64 %27
  store i64 %35, ptr %41, align 8
  %42 = add nuw nsw i64 %27, 1
  %43 = icmp ult i64 %37, %5
  %44 = icmp slt i64 %40, %8
  %45 = select i1 %43, i1 %44, i1 false
  br i1 %45, label %26, label %13, !llvm.loop !10

46:                                               ; preds = %18, %13
  %47 = phi i64 [ %16, %13 ], [ %25, %18 ]
  %48 = icmp slt i64 %15, %8
  br i1 %48, label %49, label %57

49:                                               ; preds = %46
  %50 = shl i64 %47, 3
  %51 = getelementptr i8, ptr %0, i64 %50
  %52 = shl i64 %15, 3
  %53 = getelementptr i8, ptr %10, i64 %52
  %54 = add i64 %15, %5
  %55 = sub i64 %1, %54
  %56 = shl nuw i64 %55, 3
  tail call void @llvm.memcpy.p0.p0.i64(ptr align 8 %51, ptr align 8 %53, i64 %56, i1 false), !tbaa !6
  br label %57

57:                                               ; preds = %49, %46
  tail call void @free(ptr noundef %7)
  tail call void @free(ptr noundef %10)
  br label %58

58:                                               ; preds = %2, %57
  ret void
}

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #1

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #2

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %40, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !12
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = shl i64 %7, 3
  %9 = tail call ptr @malloc(i64 noundef %8) #7
  %10 = icmp sgt i64 %7, 0
  br i1 %10, label %13, label %11

11:                                               ; preds = %4
  tail call void @merge_sort(ptr noundef %9, i64 noundef %7)
  br label %24

12:                                               ; preds = %13
  tail call void @merge_sort(ptr noundef nonnull %9, i64 noundef %7)
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
  br i1 %23, label %12, label %13, !llvm.loop !14

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
  br i1 %39, label %24, label %31, !llvm.loop !15

40:                                               ; preds = %2, %24
  %41 = phi i32 [ 0, %24 ], [ 1, %2 ]
  ret i32 %41
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #3

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #4

; Function Attrs: nocallback nofree nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memcpy.p0.p0.i64(ptr noalias nocapture writeonly, ptr noalias nocapture readonly, i64, i1 immarg) #5

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i64 @llvm.smin.i64(i64, i64) #6

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { nocallback nofree nounwind willreturn memory(argmem: readwrite) }
attributes #6 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #7 = { allocsize(0) }
attributes #8 = { nounwind }

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
