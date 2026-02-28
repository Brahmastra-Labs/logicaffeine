; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/prefix_sum/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/prefix_sum/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [5 x i8] c"%ld\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %43, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !6
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = shl i64 %7, 3
  %9 = tail call ptr @malloc(i64 noundef %8) #5
  %10 = icmp sgt i64 %7, 0
  br i1 %10, label %15, label %29

11:                                               ; preds = %15
  %12 = icmp sgt i64 %7, 1
  br i1 %12, label %13, label %29

13:                                               ; preds = %11
  %14 = load i64, ptr %9, align 8, !tbaa !10
  br label %34

15:                                               ; preds = %4, %15
  %16 = phi i64 [ %27, %15 ], [ 0, %4 ]
  %17 = phi i64 [ %20, %15 ], [ 42, %4 ]
  %18 = mul nuw nsw i64 %17, 1103515245
  %19 = add nuw nsw i64 %18, 12345
  %20 = and i64 %19, 2147483647
  %21 = lshr i64 %19, 16
  %22 = trunc i64 %21 to i16
  %23 = and i16 %22, 32767
  %24 = urem i16 %23, 1000
  %25 = zext i16 %24 to i64
  %26 = getelementptr inbounds i64, ptr %9, i64 %16
  store i64 %25, ptr %26, align 8, !tbaa !10
  %27 = add nuw nsw i64 %16, 1
  %28 = icmp eq i64 %27, %7
  br i1 %28, label %11, label %15, !llvm.loop !12

29:                                               ; preds = %34, %4, %11
  %30 = add nsw i64 %7, -1
  %31 = getelementptr inbounds i64, ptr %9, i64 %30
  %32 = load i64, ptr %31, align 8, !tbaa !10
  %33 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %32)
  tail call void @free(ptr noundef %9)
  br label %43

34:                                               ; preds = %13, %34
  %35 = phi i64 [ %40, %34 ], [ %14, %13 ]
  %36 = phi i64 [ %41, %34 ], [ 1, %13 ]
  %37 = getelementptr inbounds i64, ptr %9, i64 %36
  %38 = load i64, ptr %37, align 8, !tbaa !10
  %39 = add nsw i64 %35, %38
  %40 = srem i64 %39, 1000000007
  store i64 %40, ptr %37, align 8, !tbaa !10
  %41 = add nuw nsw i64 %36, 1
  %42 = icmp eq i64 %41, %7
  br i1 %42, label %29, label %34, !llvm.loop !14

43:                                               ; preds = %2, %29
  %44 = phi i32 [ 0, %29 ], [ 1, %2 ]
  ret i32 %44
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
