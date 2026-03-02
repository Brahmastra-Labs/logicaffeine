; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/coins/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/coins/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [5 x i8] c"%ld\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %154, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !6
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = add i64 %7, 1
  %9 = tail call ptr @calloc(i64 noundef %8, i64 noundef 8) #5
  store i64 1, ptr %9, align 8, !tbaa !10
  %10 = icmp slt i64 %7, 1
  br i1 %10, label %141, label %11

11:                                               ; preds = %4
  %12 = load i64, ptr %9, align 8
  br label %145

13:                                               ; preds = %145
  %14 = icmp slt i64 %7, 5
  br i1 %14, label %141, label %15

15:                                               ; preds = %13, %15
  %16 = phi i64 [ %24, %15 ], [ 5, %13 ]
  %17 = getelementptr inbounds i64, ptr %9, i64 %16
  %18 = load i64, ptr %17, align 8, !tbaa !10
  %19 = add nsw i64 %16, -5
  %20 = getelementptr inbounds i64, ptr %9, i64 %19
  %21 = load i64, ptr %20, align 8, !tbaa !10
  %22 = add nsw i64 %21, %18
  %23 = srem i64 %22, 1000000007
  store i64 %23, ptr %17, align 8, !tbaa !10
  %24 = add nuw i64 %16, 1
  %25 = icmp eq i64 %16, %7
  br i1 %25, label %26, label %15, !llvm.loop !12

26:                                               ; preds = %15
  %27 = icmp slt i64 %7, 10
  br i1 %27, label %141, label %28

28:                                               ; preds = %26
  %29 = add i64 %7, -9
  %30 = icmp ult i64 %29, 2
  br i1 %30, label %47, label %31

31:                                               ; preds = %28
  %32 = and i64 %29, -2
  %33 = add i64 %32, 10
  br label %34

34:                                               ; preds = %34, %31
  %35 = phi i64 [ 0, %31 ], [ %43, %34 ]
  %36 = add i64 %35, 10
  %37 = getelementptr inbounds i64, ptr %9, i64 %36
  %38 = load <2 x i64>, ptr %37, align 8, !tbaa !10
  %39 = getelementptr inbounds i64, ptr %9, i64 %35
  %40 = load <2 x i64>, ptr %39, align 8, !tbaa !10
  %41 = add nsw <2 x i64> %40, %38
  %42 = srem <2 x i64> %41, <i64 1000000007, i64 1000000007>
  store <2 x i64> %42, ptr %37, align 8, !tbaa !10
  %43 = add nuw i64 %35, 2
  %44 = icmp eq i64 %43, %32
  br i1 %44, label %45, label %34, !llvm.loop !14

45:                                               ; preds = %34
  %46 = icmp eq i64 %29, %32
  br i1 %46, label %60, label %47

47:                                               ; preds = %28, %45
  %48 = phi i64 [ 10, %28 ], [ %33, %45 ]
  br label %49

49:                                               ; preds = %47, %49
  %50 = phi i64 [ %58, %49 ], [ %48, %47 ]
  %51 = getelementptr inbounds i64, ptr %9, i64 %50
  %52 = load i64, ptr %51, align 8, !tbaa !10
  %53 = add nsw i64 %50, -10
  %54 = getelementptr inbounds i64, ptr %9, i64 %53
  %55 = load i64, ptr %54, align 8, !tbaa !10
  %56 = add nsw i64 %55, %52
  %57 = srem i64 %56, 1000000007
  store i64 %57, ptr %51, align 8, !tbaa !10
  %58 = add nuw i64 %50, 1
  %59 = icmp eq i64 %50, %7
  br i1 %59, label %60, label %49, !llvm.loop !17

60:                                               ; preds = %49, %45
  %61 = icmp slt i64 %7, 25
  br i1 %61, label %141, label %62

62:                                               ; preds = %60, %62
  %63 = phi i64 [ %71, %62 ], [ 25, %60 ]
  %64 = getelementptr inbounds i64, ptr %9, i64 %63
  %65 = load i64, ptr %64, align 8, !tbaa !10
  %66 = add nsw i64 %63, -25
  %67 = getelementptr inbounds i64, ptr %9, i64 %66
  %68 = load i64, ptr %67, align 8, !tbaa !10
  %69 = add nsw i64 %68, %65
  %70 = srem i64 %69, 1000000007
  store i64 %70, ptr %64, align 8, !tbaa !10
  %71 = add nuw i64 %63, 1
  %72 = icmp eq i64 %63, %7
  br i1 %72, label %73, label %62, !llvm.loop !12

73:                                               ; preds = %62
  %74 = icmp slt i64 %7, 50
  br i1 %74, label %141, label %75

75:                                               ; preds = %73
  %76 = add i64 %7, -49
  %77 = icmp ult i64 %76, 2
  br i1 %77, label %94, label %78

78:                                               ; preds = %75
  %79 = and i64 %76, -2
  %80 = add i64 %79, 50
  br label %81

81:                                               ; preds = %81, %78
  %82 = phi i64 [ 0, %78 ], [ %90, %81 ]
  %83 = add i64 %82, 50
  %84 = getelementptr inbounds i64, ptr %9, i64 %83
  %85 = load <2 x i64>, ptr %84, align 8, !tbaa !10
  %86 = getelementptr inbounds i64, ptr %9, i64 %82
  %87 = load <2 x i64>, ptr %86, align 8, !tbaa !10
  %88 = add nsw <2 x i64> %87, %85
  %89 = srem <2 x i64> %88, <i64 1000000007, i64 1000000007>
  store <2 x i64> %89, ptr %84, align 8, !tbaa !10
  %90 = add nuw i64 %82, 2
  %91 = icmp eq i64 %90, %79
  br i1 %91, label %92, label %81, !llvm.loop !18

92:                                               ; preds = %81
  %93 = icmp eq i64 %76, %79
  br i1 %93, label %107, label %94

94:                                               ; preds = %75, %92
  %95 = phi i64 [ 50, %75 ], [ %80, %92 ]
  br label %96

96:                                               ; preds = %94, %96
  %97 = phi i64 [ %105, %96 ], [ %95, %94 ]
  %98 = getelementptr inbounds i64, ptr %9, i64 %97
  %99 = load i64, ptr %98, align 8, !tbaa !10
  %100 = add nsw i64 %97, -50
  %101 = getelementptr inbounds i64, ptr %9, i64 %100
  %102 = load i64, ptr %101, align 8, !tbaa !10
  %103 = add nsw i64 %102, %99
  %104 = srem i64 %103, 1000000007
  store i64 %104, ptr %98, align 8, !tbaa !10
  %105 = add nuw i64 %97, 1
  %106 = icmp eq i64 %97, %7
  br i1 %106, label %107, label %96, !llvm.loop !19

107:                                              ; preds = %96, %92
  %108 = icmp slt i64 %7, 100
  br i1 %108, label %141, label %109

109:                                              ; preds = %107
  %110 = add i64 %7, -99
  %111 = icmp ult i64 %110, 2
  br i1 %111, label %128, label %112

112:                                              ; preds = %109
  %113 = and i64 %110, -2
  %114 = add i64 %113, 100
  br label %115

115:                                              ; preds = %115, %112
  %116 = phi i64 [ 0, %112 ], [ %124, %115 ]
  %117 = add i64 %116, 100
  %118 = getelementptr inbounds i64, ptr %9, i64 %117
  %119 = load <2 x i64>, ptr %118, align 8, !tbaa !10
  %120 = getelementptr inbounds i64, ptr %9, i64 %116
  %121 = load <2 x i64>, ptr %120, align 8, !tbaa !10
  %122 = add nsw <2 x i64> %121, %119
  %123 = srem <2 x i64> %122, <i64 1000000007, i64 1000000007>
  store <2 x i64> %123, ptr %118, align 8, !tbaa !10
  %124 = add nuw i64 %116, 2
  %125 = icmp eq i64 %124, %113
  br i1 %125, label %126, label %115, !llvm.loop !20

126:                                              ; preds = %115
  %127 = icmp eq i64 %110, %113
  br i1 %127, label %141, label %128

128:                                              ; preds = %109, %126
  %129 = phi i64 [ 100, %109 ], [ %114, %126 ]
  br label %130

130:                                              ; preds = %128, %130
  %131 = phi i64 [ %139, %130 ], [ %129, %128 ]
  %132 = getelementptr inbounds i64, ptr %9, i64 %131
  %133 = load i64, ptr %132, align 8, !tbaa !10
  %134 = add nsw i64 %131, -100
  %135 = getelementptr inbounds i64, ptr %9, i64 %134
  %136 = load i64, ptr %135, align 8, !tbaa !10
  %137 = add nsw i64 %136, %133
  %138 = srem i64 %137, 1000000007
  store i64 %138, ptr %132, align 8, !tbaa !10
  %139 = add nuw i64 %131, 1
  %140 = icmp eq i64 %131, %7
  br i1 %140, label %141, label %130, !llvm.loop !21

141:                                              ; preds = %130, %126, %4, %13, %26, %60, %73, %107
  %142 = getelementptr inbounds i64, ptr %9, i64 %7
  %143 = load i64, ptr %142, align 8, !tbaa !10
  %144 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %143)
  tail call void @free(ptr noundef nonnull %9)
  br label %154

145:                                              ; preds = %11, %145
  %146 = phi i64 [ %12, %11 ], [ %151, %145 ]
  %147 = phi i64 [ 1, %11 ], [ %152, %145 ]
  %148 = getelementptr inbounds i64, ptr %9, i64 %147
  %149 = load i64, ptr %148, align 8, !tbaa !10
  %150 = add nsw i64 %146, %149
  %151 = srem i64 %150, 1000000007
  store i64 %151, ptr %148, align 8, !tbaa !10
  %152 = add nuw i64 %147, 1
  %153 = icmp eq i64 %147, %7
  br i1 %153, label %13, label %145, !llvm.loop !12

154:                                              ; preds = %2, %141
  %155 = phi i32 [ 0, %141 ], [ 1, %2 ]
  ret i32 %155
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #1

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,zeroed") allocsize(0,1) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @calloc(i64 noundef, i64 noundef) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #4

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn allockind("alloc,zeroed") allocsize(0,1) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { allocsize(0,1) }

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
!14 = distinct !{!14, !13, !15, !16}
!15 = !{!"llvm.loop.isvectorized", i32 1}
!16 = !{!"llvm.loop.unroll.runtime.disable"}
!17 = distinct !{!17, !13, !16, !15}
!18 = distinct !{!18, !13, !15, !16}
!19 = distinct !{!19, !13, !16, !15}
!20 = distinct !{!20, !13, !15, !16}
!21 = distinct !{!21, !13, !16, !15}
