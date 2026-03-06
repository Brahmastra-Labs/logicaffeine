; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/string_search/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/string_search/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [6 x i8] c"XXXXX\00", align 1
@.str.1 = private unnamed_addr constant [5 x i8] c"%ld\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %72, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !6
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = add nsw i64 %7, 6
  %9 = tail call ptr @malloc(i64 noundef %8) #6
  %10 = icmp sgt i64 %7, 0
  br i1 %10, label %13, label %11

11:                                               ; preds = %4
  %12 = getelementptr inbounds i8, ptr %9, i64 %7
  store i8 0, ptr %12, align 1, !tbaa !10
  br label %45

13:                                               ; preds = %4, %30
  %14 = phi i64 [ %31, %30 ], [ 0, %4 ]
  %15 = icmp sgt i64 %14, 0
  %16 = urem i64 %14, 1000
  %17 = icmp eq i64 %16, 0
  %18 = and i1 %15, %17
  br i1 %18, label %19, label %24

19:                                               ; preds = %13
  %20 = add nuw nsw i64 %14, 5
  %21 = icmp sgt i64 %20, %7
  br i1 %21, label %24, label %22

22:                                               ; preds = %19
  %23 = getelementptr inbounds i8, ptr %9, i64 %14
  tail call void @llvm.memcpy.p0.p0.i64(ptr noundef nonnull align 1 dereferenceable(5) %23, ptr noundef nonnull align 1 dereferenceable(5) @.str, i64 noundef 5, i1 noundef false) #7
  br label %30

24:                                               ; preds = %19, %13
  %25 = srem i64 %14, 5
  %26 = trunc i64 %25 to i8
  %27 = add nsw i8 %26, 97
  %28 = getelementptr inbounds i8, ptr %9, i64 %14
  store i8 %27, ptr %28, align 1, !tbaa !10
  %29 = add nsw i64 %14, 1
  br label %30

30:                                               ; preds = %24, %22
  %31 = phi i64 [ %20, %22 ], [ %29, %24 ]
  %32 = icmp slt i64 %31, %7
  br i1 %32, label %13, label %33, !llvm.loop !11

33:                                               ; preds = %30
  %34 = getelementptr inbounds i8, ptr %9, i64 %7
  store i8 0, ptr %34, align 1, !tbaa !10
  %35 = icmp slt i64 %7, 5
  br i1 %35, label %45, label %36

36:                                               ; preds = %33
  %37 = add i64 %7, -5
  br label %38

38:                                               ; preds = %36, %68
  %39 = phi i64 [ %44, %68 ], [ 0, %36 ]
  %40 = phi i64 [ %70, %68 ], [ 0, %36 ]
  %41 = getelementptr inbounds i8, ptr %9, i64 %39
  %42 = load i8, ptr %41, align 1, !tbaa !10
  %43 = icmp eq i8 %42, 88
  %44 = add nuw i64 %39, 1
  br i1 %43, label %48, label %68

45:                                               ; preds = %68, %11, %33
  %46 = phi i64 [ 0, %33 ], [ 0, %11 ], [ %70, %68 ]
  %47 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str.1, i64 noundef %46)
  tail call void @free(ptr noundef nonnull %9)
  br label %72

48:                                               ; preds = %38
  %49 = getelementptr inbounds i8, ptr %9, i64 %44
  %50 = load i8, ptr %49, align 1, !tbaa !10
  %51 = icmp eq i8 %50, 88
  br i1 %51, label %52, label %68

52:                                               ; preds = %48
  %53 = add nuw nsw i64 %39, 2
  %54 = getelementptr inbounds i8, ptr %9, i64 %53
  %55 = load i8, ptr %54, align 1, !tbaa !10
  %56 = icmp eq i8 %55, 88
  br i1 %56, label %57, label %68

57:                                               ; preds = %52
  %58 = add nuw nsw i64 %39, 3
  %59 = getelementptr inbounds i8, ptr %9, i64 %58
  %60 = load i8, ptr %59, align 1, !tbaa !10
  %61 = icmp eq i8 %60, 88
  br i1 %61, label %62, label %68

62:                                               ; preds = %57
  %63 = add nuw nsw i64 %39, 4
  %64 = getelementptr inbounds i8, ptr %9, i64 %63
  %65 = load i8, ptr %64, align 1, !tbaa !10
  %66 = icmp eq i8 %65, 88
  %67 = zext i1 %66 to i64
  br label %68

68:                                               ; preds = %62, %38, %57, %52, %48
  %69 = phi i64 [ 0, %57 ], [ 0, %52 ], [ 0, %48 ], [ 0, %38 ], [ %67, %62 ]
  %70 = add nuw nsw i64 %40, %69
  %71 = icmp eq i64 %39, %37
  br i1 %71, label %45, label %38, !llvm.loop !13

72:                                               ; preds = %2, %45
  %73 = phi i32 [ 0, %45 ], [ 1, %2 ]
  ret i32 %73
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #1

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #4

; Function Attrs: nocallback nofree nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memcpy.p0.p0.i64(ptr noalias nocapture writeonly, ptr noalias nocapture readonly, i64, i1 immarg) #5

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { nocallback nofree nounwind willreturn memory(argmem: readwrite) }
attributes #6 = { allocsize(0) }
attributes #7 = { nounwind }

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
!10 = !{!8, !8, i64 0}
!11 = distinct !{!11, !12}
!12 = !{!"llvm.loop.mustprogress"}
!13 = distinct !{!13, !12}
