; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/collect/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/collect/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

%struct.Entry = type { i32, i32, i32 }

@__stderrp = external local_unnamed_addr global ptr, align 8
@.str = private unnamed_addr constant [20 x i8] c"Usage: collect <n>\0A\00", align 1
@table = internal unnamed_addr global ptr null, align 8
@.str.1 = private unnamed_addr constant [4 x i8] c"%d\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %4, label %7

4:                                                ; preds = %2
  %5 = load ptr, ptr @__stderrp, align 8, !tbaa !6
  %6 = tail call i64 @fwrite(ptr nonnull @.str, i64 19, i64 1, ptr %5)
  br label %107

7:                                                ; preds = %2
  %8 = getelementptr inbounds ptr, ptr %1, i64 1
  %9 = load ptr, ptr %8, align 8, !tbaa !6
  %10 = tail call i32 @atoi(ptr nocapture noundef %9)
  %11 = shl nsw i32 %10, 1
  %12 = add i32 %11, -1
  %13 = lshr i32 %12, 1
  %14 = or i32 %13, %12
  %15 = lshr i32 %14, 2
  %16 = or i32 %15, %14
  %17 = lshr i32 %16, 4
  %18 = or i32 %17, %16
  %19 = lshr i32 %18, 8
  %20 = or i32 %19, %18
  %21 = lshr i32 %20, 16
  %22 = or i32 %21, %20
  %23 = add i32 %22, 1
  %24 = tail call i32 @llvm.umax.i32(i32 %23, i32 16)
  %25 = add i32 %24, -1
  %26 = zext i32 %24 to i64
  %27 = tail call ptr @calloc(i64 noundef %26, i64 noundef 12) #7
  store ptr %27, ptr @table, align 8, !tbaa !6
  %28 = icmp sgt i32 %10, 0
  br i1 %28, label %30, label %66

29:                                               ; preds = %59
  br i1 %28, label %70, label %66

30:                                               ; preds = %7, %59
  %31 = phi i32 [ %64, %59 ], [ 0, %7 ]
  %32 = shl nuw nsw i32 %31, 1
  %33 = lshr i32 %31, 16
  %34 = xor i32 %33, %31
  %35 = mul i32 %34, 73244475
  %36 = lshr i32 %35, 16
  %37 = xor i32 %36, %35
  %38 = and i32 %37, %25
  %39 = zext i32 %38 to i64
  %40 = getelementptr inbounds %struct.Entry, ptr %27, i64 %39, i32 2
  %41 = load i32, ptr %40, align 4, !tbaa !10
  %42 = icmp eq i32 %41, 0
  br i1 %42, label %59, label %43

43:                                               ; preds = %30
  %44 = getelementptr inbounds %struct.Entry, ptr %27, i64 %39
  %45 = load i32, ptr %44, align 4, !tbaa !13
  %46 = icmp eq i32 %45, %31
  br i1 %46, label %59, label %51

47:                                               ; preds = %51
  %48 = getelementptr inbounds %struct.Entry, ptr %27, i64 %55
  %49 = load i32, ptr %48, align 4, !tbaa !13
  %50 = icmp eq i32 %49, %31
  br i1 %50, label %59, label %51, !llvm.loop !14

51:                                               ; preds = %43, %47
  %52 = phi i32 [ %54, %47 ], [ %38, %43 ]
  %53 = add i32 %52, 1
  %54 = and i32 %53, %25
  %55 = zext i32 %54 to i64
  %56 = getelementptr inbounds %struct.Entry, ptr %27, i64 %55, i32 2
  %57 = load i32, ptr %56, align 4, !tbaa !10
  %58 = icmp eq i32 %57, 0
  br i1 %58, label %59, label %47, !llvm.loop !14

59:                                               ; preds = %51, %47, %43, %30
  %60 = phi i64 [ %39, %30 ], [ %39, %43 ], [ %55, %47 ], [ %55, %51 ]
  %61 = phi ptr [ %40, %30 ], [ %40, %43 ], [ %56, %47 ], [ %56, %51 ]
  %62 = getelementptr inbounds %struct.Entry, ptr %27, i64 %60
  store i32 %31, ptr %62, align 4, !tbaa !13
  %63 = getelementptr inbounds %struct.Entry, ptr %27, i64 %60, i32 1
  store i32 %32, ptr %63, align 4, !tbaa !16
  store i32 1, ptr %61, align 4, !tbaa !10
  %64 = add nuw nsw i32 %31, 1
  %65 = icmp eq i32 %64, %10
  br i1 %65, label %29, label %30, !llvm.loop !17

66:                                               ; preds = %99, %7, %29
  %67 = phi i32 [ 0, %29 ], [ 0, %7 ], [ %104, %99 ]
  %68 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str.1, i32 noundef %67)
  %69 = load ptr, ptr @table, align 8, !tbaa !6
  tail call void @free(ptr noundef %69)
  br label %107

70:                                               ; preds = %29, %99
  %71 = phi i32 [ %105, %99 ], [ 0, %29 ]
  %72 = phi i32 [ %104, %99 ], [ 0, %29 ]
  %73 = lshr i32 %71, 16
  %74 = xor i32 %73, %71
  %75 = mul i32 %74, 73244475
  %76 = lshr i32 %75, 16
  %77 = xor i32 %76, %75
  %78 = and i32 %77, %25
  %79 = zext i32 %78 to i64
  %80 = getelementptr inbounds %struct.Entry, ptr %27, i64 %79, i32 2
  %81 = load i32, ptr %80, align 4, !tbaa !10
  %82 = icmp eq i32 %81, 0
  br i1 %82, label %99, label %83

83:                                               ; preds = %70, %92
  %84 = phi i64 [ %95, %92 ], [ %79, %70 ]
  %85 = phi i32 [ %94, %92 ], [ %78, %70 ]
  %86 = getelementptr inbounds %struct.Entry, ptr %27, i64 %84
  %87 = load i32, ptr %86, align 4, !tbaa !13
  %88 = icmp eq i32 %87, %71
  br i1 %88, label %89, label %92

89:                                               ; preds = %83
  %90 = getelementptr inbounds %struct.Entry, ptr %27, i64 %84, i32 1
  %91 = load i32, ptr %90, align 4, !tbaa !16
  br label %99

92:                                               ; preds = %83
  %93 = add i32 %85, 1
  %94 = and i32 %93, %25
  %95 = zext i32 %94 to i64
  %96 = getelementptr inbounds %struct.Entry, ptr %27, i64 %95, i32 2
  %97 = load i32, ptr %96, align 4, !tbaa !10
  %98 = icmp eq i32 %97, 0
  br i1 %98, label %99, label %83, !llvm.loop !18

99:                                               ; preds = %92, %70, %89
  %100 = phi i32 [ %91, %89 ], [ -1, %70 ], [ -1, %92 ]
  %101 = shl nuw nsw i32 %71, 1
  %102 = icmp eq i32 %100, %101
  %103 = zext i1 %102 to i32
  %104 = add nuw nsw i32 %72, %103
  %105 = add nuw nsw i32 %71, 1
  %106 = icmp eq i32 %105, %10
  br i1 %106, label %66, label %70, !llvm.loop !19

107:                                              ; preds = %66, %4
  %108 = phi i32 [ 1, %4 ], [ 0, %66 ]
  ret i32 %108
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i32 @atoi(ptr nocapture noundef) local_unnamed_addr #1

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,zeroed") allocsize(0,1) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @calloc(i64 noundef, i64 noundef) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #4

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i32 @llvm.umax.i32(i32, i32) #5

; Function Attrs: nofree nounwind
declare noundef i64 @fwrite(ptr nocapture noundef, i64 noundef, i64 noundef, ptr nocapture noundef) local_unnamed_addr #6

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn allockind("alloc,zeroed") allocsize(0,1) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #6 = { nofree nounwind }
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
!10 = !{!11, !12, i64 8}
!11 = !{!"Entry", !12, i64 0, !12, i64 4, !12, i64 8}
!12 = !{!"int", !8, i64 0}
!13 = !{!11, !12, i64 0}
!14 = distinct !{!14, !15}
!15 = !{!"llvm.loop.mustprogress"}
!16 = !{!11, !12, i64 4}
!17 = distinct !{!17, !15}
!18 = distinct !{!18, !15}
!19 = distinct !{!19, !15}
