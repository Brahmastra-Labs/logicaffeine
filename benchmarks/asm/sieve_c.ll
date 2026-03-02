; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/sieve/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/sieve/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@__stderrp = external local_unnamed_addr global ptr, align 8
@.str = private unnamed_addr constant [22 x i8] c"Usage: sieve <limit>\0A\00", align 1
@.str.1 = private unnamed_addr constant [4 x i8] c"%d\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %4, label %7

4:                                                ; preds = %2
  %5 = load ptr, ptr @__stderrp, align 8, !tbaa !6
  %6 = tail call i64 @fwrite(ptr nonnull @.str, i64 21, i64 1, ptr %5)
  br label %86

7:                                                ; preds = %2
  %8 = getelementptr inbounds ptr, ptr %1, i64 1
  %9 = load ptr, ptr %8, align 8, !tbaa !6
  %10 = tail call i32 @atoi(ptr nocapture noundef %9)
  %11 = add i32 %10, 1
  %12 = sext i32 %11 to i64
  %13 = tail call ptr @calloc(i64 noundef %12, i64 noundef 1) #7
  %14 = icmp eq ptr %13, null
  br i1 %14, label %86, label %15

15:                                               ; preds = %7
  %16 = icmp slt i32 %10, 2
  br i1 %16, label %21, label %17

17:                                               ; preds = %15
  %18 = zext i32 %10 to i64
  %19 = zext i32 %11 to i64
  %20 = add nuw nsw i64 %18, 1
  br label %24

21:                                               ; preds = %78, %15
  %22 = phi i32 [ 0, %15 ], [ %79, %78 ]
  %23 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str.1, i32 noundef %22)
  tail call void @free(ptr noundef nonnull %13)
  br label %86

24:                                               ; preds = %17, %78
  %25 = phi i64 [ -6, %17 ], [ %85, %78 ]
  %26 = phi i64 [ 6, %17 ], [ %83, %78 ]
  %27 = phi i64 [ 0, %17 ], [ %82, %78 ]
  %28 = phi i64 [ 2, %17 ], [ %80, %78 ]
  %29 = phi i32 [ 0, %17 ], [ %79, %78 ]
  %30 = shl i64 %27, 1
  %31 = shl nuw nsw i64 %27, 1
  %32 = add nuw i64 %31, 6
  %33 = tail call i64 @llvm.umax.i64(i64 %26, i64 %20)
  %34 = add i64 %33, %25
  %35 = icmp ne i64 %34, 0
  %36 = sext i1 %35 to i64
  %37 = add i64 %34, %36
  %38 = getelementptr inbounds i8, ptr %13, i64 %28
  %39 = load i8, ptr %38, align 1, !tbaa !10
  %40 = icmp eq i8 %39, 0
  br i1 %40, label %41, label %78

41:                                               ; preds = %24
  %42 = add nsw i32 %29, 1
  %43 = mul nuw nsw i64 %28, %28
  %44 = icmp ugt i64 %43, %18
  br i1 %44, label %78, label %45

45:                                               ; preds = %41
  %46 = select i1 %35, i64 2, i64 1
  %47 = udiv i64 %37, %28
  %48 = add i64 %46, %47
  %49 = icmp ult i64 %48, 4
  br i1 %49, label %71, label %50

50:                                               ; preds = %45
  %51 = and i64 %48, -4
  %52 = add i64 %28, %51
  %53 = mul i64 %28, %52
  br label %54

54:                                               ; preds = %54, %50
  %55 = phi i64 [ 0, %50 ], [ %67, %54 ]
  %56 = add i64 %28, %55
  %57 = mul i64 %28, %56
  %58 = add i64 %57, %28
  %59 = add i64 %56, 2
  %60 = mul i64 %28, %59
  %61 = add i64 %56, 3
  %62 = mul i64 %28, %61
  %63 = getelementptr inbounds i8, ptr %13, i64 %57
  %64 = getelementptr inbounds i8, ptr %13, i64 %58
  %65 = getelementptr inbounds i8, ptr %13, i64 %60
  %66 = getelementptr inbounds i8, ptr %13, i64 %62
  store i8 1, ptr %63, align 1, !tbaa !10
  store i8 1, ptr %64, align 1, !tbaa !10
  store i8 1, ptr %65, align 1, !tbaa !10
  store i8 1, ptr %66, align 1, !tbaa !10
  %67 = add nuw i64 %55, 4
  %68 = icmp eq i64 %67, %51
  br i1 %68, label %69, label %54, !llvm.loop !11

69:                                               ; preds = %54
  %70 = icmp eq i64 %48, %51
  br i1 %70, label %78, label %71

71:                                               ; preds = %45, %69
  %72 = phi i64 [ %43, %45 ], [ %53, %69 ]
  br label %73

73:                                               ; preds = %71, %73
  %74 = phi i64 [ %76, %73 ], [ %72, %71 ]
  %75 = getelementptr inbounds i8, ptr %13, i64 %74
  store i8 1, ptr %75, align 1, !tbaa !10
  %76 = add nuw nsw i64 %74, %28
  %77 = icmp ugt i64 %76, %18
  br i1 %77, label %78, label %73, !llvm.loop !15

78:                                               ; preds = %73, %69, %41, %24
  %79 = phi i32 [ %29, %24 ], [ %42, %41 ], [ %42, %69 ], [ %42, %73 ]
  %80 = add nuw nsw i64 %28, 1
  %81 = icmp eq i64 %80, %19
  %82 = add i64 %27, 1
  %83 = add i64 %26, %32
  %84 = sub i64 %25, %30
  %85 = add i64 %84, -6
  br i1 %81, label %21, label %24, !llvm.loop !16

86:                                               ; preds = %21, %7, %4
  %87 = phi i32 [ 1, %4 ], [ 0, %21 ], [ 1, %7 ]
  ret i32 %87
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i32 @atoi(ptr nocapture noundef) local_unnamed_addr #1

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,zeroed") allocsize(0,1) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @calloc(i64 noundef, i64 noundef) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #4

; Function Attrs: nofree nounwind
declare noundef i64 @fwrite(ptr nocapture noundef, i64 noundef, i64 noundef, ptr nocapture noundef) local_unnamed_addr #5

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i64 @llvm.umax.i64(i64, i64) #6

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn allockind("alloc,zeroed") allocsize(0,1) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { nofree nounwind }
attributes #6 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #7 = { allocsize(0,1) }

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
!11 = distinct !{!11, !12, !13, !14}
!12 = !{!"llvm.loop.mustprogress"}
!13 = !{!"llvm.loop.isvectorized", i32 1}
!14 = !{!"llvm.loop.unroll.runtime.disable"}
!15 = distinct !{!15, !12, !13}
!16 = distinct !{!16, !12}
