; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/array_reverse/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/array_reverse/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [13 x i8] c"%ld %ld %ld\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %46, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !6
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = shl i64 %7, 3
  %9 = tail call ptr @malloc(i64 noundef %8) #5
  %10 = icmp sgt i64 %7, 0
  br i1 %10, label %16, label %11

11:                                               ; preds = %4
  %12 = add nsw i64 %7, -1
  br label %37

13:                                               ; preds = %16
  %14 = add nsw i64 %7, -1
  %15 = icmp sgt i64 %7, 1
  br i1 %15, label %27, label %37

16:                                               ; preds = %4, %16
  %17 = phi i64 [ %25, %16 ], [ 0, %4 ]
  %18 = phi i64 [ %21, %16 ], [ 42, %4 ]
  %19 = mul nuw nsw i64 %18, 1103515245
  %20 = add nuw nsw i64 %19, 12345
  %21 = and i64 %20, 2147483647
  %22 = lshr i64 %20, 16
  %23 = and i64 %22, 32767
  %24 = getelementptr inbounds i64, ptr %9, i64 %17
  store i64 %23, ptr %24, align 8, !tbaa !10
  %25 = add nuw nsw i64 %17, 1
  %26 = icmp eq i64 %25, %7
  br i1 %26, label %13, label %16, !llvm.loop !12

27:                                               ; preds = %13, %27
  %28 = phi i64 [ %35, %27 ], [ %14, %13 ]
  %29 = phi i64 [ %34, %27 ], [ 0, %13 ]
  %30 = getelementptr inbounds i64, ptr %9, i64 %29
  %31 = load i64, ptr %30, align 8, !tbaa !10
  %32 = getelementptr inbounds i64, ptr %9, i64 %28
  %33 = load i64, ptr %32, align 8, !tbaa !10
  store i64 %33, ptr %30, align 8, !tbaa !10
  store i64 %31, ptr %32, align 8, !tbaa !10
  %34 = add nuw nsw i64 %29, 1
  %35 = add nsw i64 %28, -1
  %36 = icmp slt i64 %34, %35
  br i1 %36, label %27, label %37, !llvm.loop !14

37:                                               ; preds = %27, %11, %13
  %38 = phi i64 [ %12, %11 ], [ %14, %13 ], [ %14, %27 ]
  %39 = load i64, ptr %9, align 8, !tbaa !10
  %40 = getelementptr inbounds i64, ptr %9, i64 %38
  %41 = load i64, ptr %40, align 8, !tbaa !10
  %42 = sdiv i64 %7, 2
  %43 = getelementptr inbounds i64, ptr %9, i64 %42
  %44 = load i64, ptr %43, align 8, !tbaa !10
  %45 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %39, i64 noundef %41, i64 noundef %44)
  tail call void @free(ptr noundef %9)
  br label %46

46:                                               ; preds = %2, %37
  %47 = phi i32 [ 0, %37 ], [ 1, %2 ]
  ret i32 %47
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #1

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #4

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { allocsize(0) }

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
!11 = !{!"long", !8, i64 0}
!12 = distinct !{!12, !13}
!13 = !{!"llvm.loop.mustprogress"}
!14 = distinct !{!14, !13}
