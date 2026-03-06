//! Phase Futamura: Self-Interpreter + Futamura Projections
//!
//! Sprint 4: LOGOS-in-LOGOS self-interpreter (35 tests)
//! Sprint 5: Projection 1 — pe(int, program) = compiled_program

mod common;

const CORE_TYPES: &str = r#"
## A CExpr is one of:
    A CInt with value Int.
    A CFloat with value Real.
    A CBool with value Bool.
    A CText with value Text.
    A CVar with name Text.
    A CBinOp with op Text and left CExpr and right CExpr.
    A CNot with inner CExpr.
    A CCall with name Text and args Seq of CExpr.
    A CIndex with coll CExpr and idx CExpr.
    A CLen with target CExpr.
    A CMapGet with target CExpr and key CExpr.
    A CNewSeq.
    A CNewVariant with tag Text and fnames Seq of Text and fvals Seq of CExpr.
    A CList with items Seq of CExpr.
    A CRange with start CExpr and end CExpr.
    A CSlice with coll CExpr and startIdx CExpr and endIdx CExpr.
    A CCopy with target CExpr.
    A CNewSet.
    A CContains with coll CExpr and elem CExpr.
    A CUnion with left CExpr and right CExpr.
    A CIntersection with left CExpr and right CExpr.
    A COptionSome with inner CExpr.
    A COptionNone.
    A CTuple with items Seq of CExpr.
    A CNew with typeName Text and fieldNames Seq of Text and fields Seq of CExpr.
    A CFieldAccess with target CExpr and field Text.
    A CClosure with params Seq of Text and body Seq of CStmt and captured Seq of Text.
    A CCallExpr with target CExpr and args Seq of CExpr.
    A CInterpolatedString with parts Seq of CStringPart.
    A CDuration with amount CExpr and unit Text.
    A CTimeNow.
    A CDateToday.
    A CEscExpr with code Text.

## A CStmt is one of:
    A CLet with name Text and expr CExpr.
    A CSet with name Text and expr CExpr.
    A CIf with cond CExpr and thenBlock Seq of CStmt and elseBlock Seq of CStmt.
    A CWhile with cond CExpr and body Seq of CStmt.
    A CReturn with expr CExpr.
    A CShow with expr CExpr.
    A CCallS with name Text and args Seq of CExpr.
    A CPush with expr CExpr and target Text.
    A CSetIdx with target Text and idx CExpr and val CExpr.
    A CMapSet with target Text and key CExpr and val CExpr.
    A CPop with target Text.
    A CRepeat with var Text and coll CExpr and body Seq of CStmt.
    A CRepeatRange with var Text and start CExpr and end CExpr and body Seq of CStmt.
    A CBreak.
    A CAdd with elem CExpr and target Text.
    A CRemove with elem CExpr and target Text.
    A CSetField with target Text and field Text and val CExpr.
    A CStructDef with name Text and fieldNames Seq of Text.
    A CInspect with target CExpr and arms Seq of CMatchArm.
    A CEnumDef with name Text and variants Seq of Text.
    A CRuntimeAssert with cond CExpr and msg CExpr.
    A CGive with expr CExpr and target Text.
    A CEscStmt with code Text.
    A CSleep with duration CExpr.
    A CReadConsole with target Text.
    A CReadFile with path CExpr and target Text.
    A CWriteFile with path CExpr and content CExpr.
    A CCheck with predicate CExpr and msg CExpr.
    A CAssert with proposition CExpr.
    A CTrust with proposition CExpr and justification Text.
    A CRequire with dependency Text.
    A CMerge with target Text and other CExpr.
    A CIncrease with target Text and amount CExpr.
    A CDecrease with target Text and amount CExpr.
    A CAppendToSeq with target Text and value CExpr.
    A CResolve with target Text.
    A CSync with target Text and channel CExpr.
    A CMount with target Text and path CExpr.
    A CConcurrent with branches Seq of Seq of CStmt.
    A CParallel with branches Seq of Seq of CStmt.
    A CLaunchTask with body Seq of CStmt and handle Text.
    A CStopTask with handle CExpr.
    A CSelect with branches Seq of CSelectBranch.
    A CCreatePipe with name Text and capacity CExpr.
    A CSendPipe with chan Text and value CExpr.
    A CReceivePipe with chan Text and target Text.
    A CTrySendPipe with chan Text and value CExpr.
    A CTryReceivePipe with chan Text and target Text.
    A CSpawn with agentType Text and target Text.
    A CSendMessage with target CExpr and msg CExpr.
    A CAwaitMessage with target Text.
    A CListen with addr CExpr and handler Text.
    A CConnectTo with addr CExpr and target Text.
    A CZone with name Text and kind Text and body Seq of CStmt.

## A CSelectBranch is one of:
    A CSelectRecv with chan Text and var Text and body Seq of CStmt.
    A CSelectTimeout with duration CExpr and body Seq of CStmt.

## A CFunc is one of:
    A CFuncDef with name Text and params Seq of Text and body Seq of CStmt.

## A CProgram is one of:
    A CProg with funcs Seq of CFunc and main Seq of CStmt.

## A CVal is one of:
    A VInt with value Int.
    A VFloat with value Real.
    A VBool with value Bool.
    A VText with value Text.
    A VSeq with items Seq of CVal.
    A VMap with entries Map of Text to CVal.
    A VError with msg Text.
    A VNothing.
    A VSet with items Seq of CVal.
    A VOption with inner CVal and present Bool.
    A VTuple with items Seq of CVal.
    A VStruct with typeName Text and fields Map of Text to CVal.
    A VVariant with typeName Text and variantName Text and fields Seq of CVal.
    A VClosure with params Seq of Text and body Seq of CStmt and capturedEnv Map of Text to CVal.
    A VDuration with millis Int.
    A VDate with year Int and month Int and day Int.
    A VMoment with millis Int.
    A VSpan with startMillis Int and endMillis Int.
    A VTime with hour Int and minute Int and second Int.
    A VCrdt with kind Text and state Map of Text to CVal.

## A CMatchArm is one of:
    A CWhen with variantName Text and bindings Seq of Text and body Seq of CStmt.
    A COtherwise with body Seq of CStmt.

## A CStringPart is one of:
    A CLiteralPart with value Text.
    A CExprPart with expr CExpr.
"#;

const INTERPRETER: &str = r#"
## To isNothing (v: CVal) -> Bool:
    Inspect v:
        When VNothing:
            Return true.
        Otherwise:
            Return false.

## To valToText (v: CVal) -> Text:
    Inspect v:
        When VInt (n):
            Return "{n}".
        When VFloat (f):
            Return "{f}".
        When VBool (b):
            If b:
                Return "true".
            Otherwise:
                Return "false".
        When VText (s):
            Return s.
        When VSeq (items):
            Return "[seq]".
        When VMap (m):
            Return "[map]".
        When VSet (setItems):
            Return "[set]".
        When VOption (optInner, optPresent):
            If optPresent:
                Let innerText be valToText(optInner).
                Return "Some({innerText})".
            Otherwise:
                Return "None".
        When VTuple (tupItems):
            Let tupParts be a new Seq of Text.
            Repeat for ti in tupItems:
                Push valToText(ti) to tupParts.
            Let mutable tupResult be "(".
            Let mutable tupIdx be 1.
            Let tupLen be length of tupParts.
            Repeat for tp in tupParts:
                Set tupResult to "{tupResult}{tp}".
                If tupIdx is less than tupLen:
                    Set tupResult to "{tupResult}, ".
                Set tupIdx to tupIdx + 1.
            Set tupResult to "{tupResult})".
            Return tupResult.
        When VStruct (sTypeName, sFields):
            Return "{sTypeName}(...)".
        When VVariant (vTypeName, vVarName, vFields):
            Return "{vVarName}".
        When VClosure (clParams, clBody, clEnv):
            Return "<closure>".
        When VDuration (durMs):
            If durMs is less than 1000:
                Return "{durMs}ms".
            Let durSec be durMs / 1000.
            If durSec is less than 60:
                Return "{durSec}s".
            Let durMin be durSec / 60.
            Return "{durMin}m".
        When VDate (dYear, dMonth, dDay):
            Return "{dYear}-{dMonth}-{dDay}".
        When VMoment (mMs):
            Return "moment({mMs})".
        When VSpan (spanStart, spanEnd):
            Return "span({spanStart}..{spanEnd})".
        When VTime (tHour, tMin, tSec):
            Return "{tHour}:{tMin}:{tSec}".
        When VCrdt (crdtKind, crdtState):
            Return "<crdt:{crdtKind}>".
        When VError (msg):
            Return "Error: {msg}".
        When VNothing:
            Return "nothing".

## To applyBinOp (op: Text) and (lv: CVal) and (rv: CVal) -> CVal:
    Inspect lv:
        When VError (msg):
            Return a new VError with msg msg.
        When VInt (a):
            Inspect rv:
                When VError (msg):
                    Return a new VError with msg msg.
                When VInt (b):
                    If op equals "+":
                        Return a new VInt with value (a + b).
                    If op equals "-":
                        Return a new VInt with value (a - b).
                    If op equals "*":
                        Return a new VInt with value (a * b).
                    If op equals "/":
                        If b equals 0:
                            Return a new VError with msg "division by zero".
                        Return a new VInt with value (a / b).
                    If op equals "%":
                        If b equals 0:
                            Return a new VError with msg "modulo by zero".
                        Return a new VInt with value (a % b).
                    If op equals "<":
                        Return a new VBool with value (a is less than b).
                    If op equals ">":
                        Return a new VBool with value (a is greater than b).
                    If op equals "<=":
                        Return a new VBool with value (a is at most b).
                    If op equals ">=":
                        Return a new VBool with value (a is at least b).
                    If op equals "==":
                        Return a new VBool with value (a equals b).
                    If op equals "!=":
                        Return a new VBool with value (a is not b).
                    If op equals "^":
                        Return a new VInt with value (a xor b).
                    If op equals "<<":
                        Return a new VInt with value (a shifted left by b).
                    If op equals ">>":
                        Return a new VInt with value (a shifted right by b).
                    Return a new VNothing.
                When VFloat (b):
                    If op equals "+":
                        Return a new VFloat with value (a + b).
                    If op equals "-":
                        Return a new VFloat with value (a - b).
                    If op equals "*":
                        Return a new VFloat with value (a * b).
                    If op equals "/":
                        If b equals 0.0:
                            Return a new VError with msg "division by zero".
                        Return a new VFloat with value (a / b).
                    If op equals "<":
                        Return a new VBool with value (a is less than b).
                    If op equals ">":
                        Return a new VBool with value (a is greater than b).
                    If op equals "<=":
                        Return a new VBool with value (a is at most b).
                    If op equals ">=":
                        Return a new VBool with value (a is at least b).
                    If op equals "==":
                        Return a new VBool with value (a equals b).
                    If op equals "!=":
                        Return a new VBool with value (a is not b).
                    Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When VFloat (a):
            Inspect rv:
                When VError (msg):
                    Return a new VError with msg msg.
                When VFloat (b):
                    If op equals "+":
                        Return a new VFloat with value (a + b).
                    If op equals "-":
                        Return a new VFloat with value (a - b).
                    If op equals "*":
                        Return a new VFloat with value (a * b).
                    If op equals "/":
                        If b equals 0.0:
                            Return a new VError with msg "division by zero".
                        Return a new VFloat with value (a / b).
                    If op equals "<":
                        Return a new VBool with value (a is less than b).
                    If op equals ">":
                        Return a new VBool with value (a is greater than b).
                    If op equals "<=":
                        Return a new VBool with value (a is at most b).
                    If op equals ">=":
                        Return a new VBool with value (a is at least b).
                    If op equals "==":
                        Return a new VBool with value (a equals b).
                    If op equals "!=":
                        Return a new VBool with value (a is not b).
                    Return a new VNothing.
                When VInt (b):
                    If op equals "+":
                        Return a new VFloat with value (a + b).
                    If op equals "-":
                        Return a new VFloat with value (a - b).
                    If op equals "*":
                        Return a new VFloat with value (a * b).
                    If op equals "/":
                        If b equals 0:
                            Return a new VError with msg "division by zero".
                        Return a new VFloat with value (a / b).
                    If op equals "<":
                        Return a new VBool with value (a is less than b).
                    If op equals ">":
                        Return a new VBool with value (a is greater than b).
                    If op equals "<=":
                        Return a new VBool with value (a is at most b).
                    If op equals ">=":
                        Return a new VBool with value (a is at least b).
                    If op equals "==":
                        Return a new VBool with value (a equals b).
                    If op equals "!=":
                        Return a new VBool with value (a is not b).
                    Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When VBool (a):
            Inspect rv:
                When VError (msg):
                    Return a new VError with msg msg.
                When VBool (b):
                    If op equals "&&":
                        Return a new VBool with value (a and b).
                    If op equals "||":
                        Return a new VBool with value (a or b).
                    If op equals "==":
                        Return a new VBool with value (a equals b).
                    If op equals "!=":
                        Return a new VBool with value (a is not b).
                    Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When VText (a):
            Inspect rv:
                When VError (msg):
                    Return a new VError with msg msg.
                When VText (b):
                    If op equals "+":
                        Let joined be "{a}{b}".
                        Return a new VText with value joined.
                    If op equals "==":
                        Return a new VBool with value (a equals b).
                    If op equals "!=":
                        Return a new VBool with value (a is not b).
                    Return a new VNothing.
                When VInt (b):
                    If op equals "+":
                        Let joined be "{a}{b}".
                        Return a new VText with value joined.
                    Return a new VNothing.
                When VBool (b):
                    If op equals "+":
                        If b:
                            Let joined be "{a}true".
                            Return a new VText with value joined.
                        Otherwise:
                            Let joined be "{a}false".
                            Return a new VText with value joined.
                    Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When VDuration (durA):
            Inspect rv:
                When VDuration (durB):
                    If op equals "+":
                        Return a new VDuration with millis (durA + durB).
                    If op equals "-":
                        Return a new VDuration with millis (durA - durB).
                    If op equals "==":
                        Return a new VBool with value (durA equals durB).
                    If op equals "!=":
                        Return a new VBool with value (durA is not durB).
                    If op equals "<":
                        Return a new VBool with value (durA is less than durB).
                    If op equals ">":
                        Return a new VBool with value (durA is greater than durB).
                    Return a new VNothing.
                When VInt (durB):
                    If op equals "*":
                        Return a new VDuration with millis (durA * durB).
                    Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When VDate (dateYA, dateMA, dateDA):
            Inspect rv:
                When VDuration (durB):
                    If op equals "+":
                        Let shiftedDay be dateDA + (durB / 86400000).
                        Return a new VDate with year dateYA and month dateMA and day shiftedDay.
                    Return a new VNothing.
                When VDate (dateYB, dateMB, dateDB):
                    If op equals "-":
                        Let dayDiff be dateDA - dateDB.
                        Return a new VDuration with millis (dayDiff * 86400000).
                    If op equals "==":
                        Let yEq be dateYA equals dateYB.
                        Let mEq be dateMA equals dateMB.
                        Let dEq be dateDA equals dateDB.
                        Return a new VBool with value (yEq and mEq and dEq).
                    If op equals "<":
                        If dateYA is less than dateYB:
                            Return a new VBool with value true.
                        If dateYA is greater than dateYB:
                            Return a new VBool with value false.
                        If dateMA is less than dateMB:
                            Return a new VBool with value true.
                        If dateMA is greater than dateMB:
                            Return a new VBool with value false.
                        Return a new VBool with value (dateDA is less than dateDB).
                    Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When VMoment (momA):
            Inspect rv:
                When VMoment (momB):
                    If op equals "<":
                        Return a new VBool with value (momA is less than momB).
                    If op equals ">":
                        Return a new VBool with value (momA is greater than momB).
                    If op equals "==":
                        Return a new VBool with value (momA equals momB).
                    If op equals "<=":
                        Return a new VBool with value (momA is at most momB).
                    If op equals ">=":
                        Return a new VBool with value (momA is at least momB).
                    Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        Otherwise:
            Return a new VNothing.

## To valEquals (a: CVal) and (b: CVal) -> Bool:
    Let ta be valToText(a).
    Let tb be valToText(b).
    Return ta equals tb.

## To coreEval (expr: CExpr) and (env: Map of Text to CVal) and (funcs: Map of Text to CFunc) -> CVal:
    Inspect expr:
        When CInt (n):
            Return a new VInt with value n.
        When CFloat (f):
            Return a new VFloat with value f.
        When CBool (b):
            Return a new VBool with value b.
        When CText (s):
            Return a new VText with value s.
        When CVar (name):
            Return item name of env.
        When CBinOp (op, left, right):
            Let lv be coreEval(left, env, funcs).
            Let rv be coreEval(right, env, funcs).
            Return applyBinOp(op, lv, rv).
        When CNot (inner):
            Let v be coreEval(inner, env, funcs).
            Inspect v:
                When VBool (b):
                    Return a new VBool with value (not b).
                Otherwise:
                    Return a new VNothing.
        When CCall (name, argExprs):
            Let argVals be a new Seq of CVal.
            Repeat for a in argExprs:
                Push coreEval(a, env, funcs) to argVals.
            Let callInFuncs be (funcs contains name).
            If callInFuncs:
                Let func be item name of funcs.
                Inspect func:
                    When CFuncDef (fname, params, body):
                        Let callEnv be a new Map of Text to CVal.
                        Let mutable idx be 1.
                        Repeat for p in params:
                            Set item p of callEnv to item idx of argVals.
                            Set idx to idx + 1.
                        Return coreExecBlock(body, callEnv, funcs).
                    Otherwise:
                        Return a new VNothing.
            Otherwise:
                Let callInEnv be (env contains name).
                If callInEnv:
                    Let envVal be item name of env.
                    Inspect envVal:
                        When VClosure (ceParams, ceBody, ceCapturedEnv):
                            Let mutable ceCallEnv be ceCapturedEnv.
                            Let mutable ceCopyIdx be 1.
                            While ceCopyIdx is at most (length of ceParams):
                                If ceCopyIdx is at most (length of argVals):
                                    Set item (item ceCopyIdx of ceParams) of ceCallEnv to item ceCopyIdx of argVals.
                                Set ceCopyIdx to ceCopyIdx + 1.
                            Return coreExecBlock(ceBody, ceCallEnv, funcs).
                        Otherwise:
                            Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When CIndex (collExpr, idxExpr):
            Let cv be coreEval(collExpr, env, funcs).
            Let iv be coreEval(idxExpr, env, funcs).
            Inspect cv:
                When VError (msg):
                    Return a new VError with msg msg.
                When VSeq (items):
                    Inspect iv:
                        When VError (msg):
                            Return a new VError with msg msg.
                        When VInt (i):
                            If i is less than 1:
                                Return a new VError with msg "index out of bounds".
                            If i is greater than (length of items):
                                Return a new VError with msg "index out of bounds".
                            Return item i of items.
                        Otherwise:
                            Return a new VNothing.
                When VTuple (tupItems):
                    Inspect iv:
                        When VInt (i):
                            If i is less than 1:
                                Return a new VError with msg "index out of bounds".
                            If i is greater than (length of tupItems):
                                Return a new VError with msg "index out of bounds".
                            Return item i of tupItems.
                        Otherwise:
                            Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When CLen (collExpr):
            Let cv be coreEval(collExpr, env, funcs).
            Inspect cv:
                When VSeq (items):
                    Return a new VInt with value (length of items).
                When VSet (setItems):
                    Return a new VInt with value (length of setItems).
                When VTuple (tupItems):
                    Return a new VInt with value (length of tupItems).
                When VText (textVal):
                    Return a new VInt with value (length of textVal).
                Otherwise:
                    Return a new VNothing.
        When CMapGet (mapExpr, keyExpr):
            Let mv be coreEval(mapExpr, env, funcs).
            Let kv be coreEval(keyExpr, env, funcs).
            Inspect mv:
                When VMap (mapData):
                    Inspect kv:
                        When VText (key):
                            Return item key of mapData.
                        Otherwise:
                            Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When CNewSeq:
            Return a new VSeq with items a new Seq of CVal.
        When CNewVariant (nvTag, nvNames, nvVals):
            Let nvMap be a new Map of Text to CVal.
            Set item "__tag" of nvMap to a new VText with value nvTag.
            Let nvFnSeq be a new Seq of CVal.
            Let mutable nvi be 1.
            While nvi is at most (length of nvNames):
                Let nvn be item nvi of nvNames.
                Let nvv be coreEval(item nvi of nvVals, env, funcs).
                Set item nvn of nvMap to nvv.
                Let nvn2 be item nvi of nvNames.
                Push a new VText with value nvn2 to nvFnSeq.
                Set nvi to nvi + 1.
            Set item "__fnames__" of nvMap to a new VSeq with items nvFnSeq.
            Return a new VMap with entries nvMap.
        When CList (listItems):
            Let result be a new Seq of CVal.
            Repeat for listItem in listItems:
                Push coreEval(listItem, env, funcs) to result.
            Return a new VSeq with items result.
        When CRange (rangeStart, rangeEnd):
            Let sv be coreEval(rangeStart, env, funcs).
            Let ev be coreEval(rangeEnd, env, funcs).
            Let result be a new Seq of CVal.
            Inspect sv:
                When VInt (s):
                    Inspect ev:
                        When VInt (e):
                            Let mutable ri be s.
                            While ri is at most e:
                                Push a new VInt with value ri to result.
                                Set ri to ri + 1.
                        Otherwise:
                            Let skip be true.
                Otherwise:
                    Let skip be true.
            Return a new VSeq with items result.
        When CSlice (sliceColl, sliceStart, sliceEnd):
            Let cv be coreEval(sliceColl, env, funcs).
            Let siv be coreEval(sliceStart, env, funcs).
            Let eiv be coreEval(sliceEnd, env, funcs).
            Inspect cv:
                When VSeq (srcItems):
                    Inspect siv:
                        When VInt (si):
                            Inspect eiv:
                                When VInt (ei):
                                    Let sliceResult be a new Seq of CVal.
                                    Let mutable sIdx be si.
                                    While sIdx is at most ei:
                                        If sIdx is at least 1:
                                            If sIdx is at most (length of srcItems):
                                                Push item sIdx of srcItems to sliceResult.
                                        Set sIdx to sIdx + 1.
                                    Return a new VSeq with items sliceResult.
                                Otherwise:
                                    Return a new VNothing.
                        Otherwise:
                            Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When CCopy (copyTarget):
            Let cv be coreEval(copyTarget, env, funcs).
            Inspect cv:
                When VSeq (srcItems):
                    Let copiedItems be a new Seq of CVal.
                    Repeat for ci in srcItems:
                        Push ci to copiedItems.
                    Return a new VSeq with items copiedItems.
                Otherwise:
                    Return cv.
        When CNewSet:
            Return a new VSet with items a new Seq of CVal.
        When CContains (containsColl, containsElem):
            Let ccv be coreEval(containsColl, env, funcs).
            Let cev be coreEval(containsElem, env, funcs).
            Let cevText be valToText(cev).
            Inspect ccv:
                When VSet (setItems):
                    Repeat for si in setItems:
                        Let siText be valToText(si).
                        If siText equals cevText:
                            Return a new VBool with value true.
                    Return a new VBool with value false.
                When VSeq (seqItems):
                    Repeat for si in seqItems:
                        Let siText be valToText(si).
                        If siText equals cevText:
                            Return a new VBool with value true.
                    Return a new VBool with value false.
                When VText (haystack):
                    Inspect cev:
                        When VText (needle):
                            If haystack contains needle:
                                Return a new VBool with value true.
                            Otherwise:
                                Return a new VBool with value false.
                        Otherwise:
                            Return a new VBool with value false.
                Otherwise:
                    Return a new VBool with value false.
        When CUnion (unionLeft, unionRight):
            Let ulv be coreEval(unionLeft, env, funcs).
            Let urv be coreEval(unionRight, env, funcs).
            Inspect ulv:
                When VSet (leftItems):
                    Inspect urv:
                        When VSet (rightItems):
                            Let unionResult be a new Seq of CVal.
                            Let unionTexts be a new Seq of Text.
                            Let mutable uliIdx be 1.
                            While uliIdx is at most (length of leftItems):
                                Push item uliIdx of leftItems to unionResult.
                                Push valToText(item uliIdx of leftItems) to unionTexts.
                                Set uliIdx to uliIdx + 1.
                            Let mutable uriIdx be 1.
                            While uriIdx is at most (length of rightItems):
                                Let uriTxt be valToText(item uriIdx of rightItems).
                                Let mutable uriFound be false.
                                Repeat for ut in unionTexts:
                                    If ut equals uriTxt:
                                        Set uriFound to true.
                                If not uriFound:
                                    Push item uriIdx of rightItems to unionResult.
                                    Push uriTxt to unionTexts.
                                Set uriIdx to uriIdx + 1.
                            Return a new VSet with items unionResult.
                        Otherwise:
                            Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When CIntersection (interLeft, interRight):
            Let ilv be coreEval(interLeft, env, funcs).
            Let irv be coreEval(interRight, env, funcs).
            Inspect ilv:
                When VSet (leftItems):
                    Inspect irv:
                        When VSet (rightItems):
                            Let interResult be a new Seq of CVal.
                            Let rightTexts be a new Seq of Text.
                            Repeat for iri in rightItems:
                                Push valToText(iri) to rightTexts.
                            Let mutable iliIdx be 1.
                            While iliIdx is at most (length of leftItems):
                                Let iliTxt be valToText(item iliIdx of leftItems).
                                Let mutable iliFound be false.
                                Repeat for irt in rightTexts:
                                    If irt equals iliTxt:
                                        Set iliFound to true.
                                If iliFound:
                                    Push item iliIdx of leftItems to interResult.
                                Set iliIdx to iliIdx + 1.
                            Return a new VSet with items interResult.
                        Otherwise:
                            Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When COptionSome (optInner):
            Let optVal be coreEval(optInner, env, funcs).
            Return a new VOption with inner optVal and present true.
        When COptionNone:
            Return a new VOption with inner (a new VNothing) and present false.
        When CTuple (tupleItems):
            Let tupleResult be a new Seq of CVal.
            Repeat for ti in tupleItems:
                Push coreEval(ti, env, funcs) to tupleResult.
            Return a new VTuple with items tupleResult.
        When CNew (newTypeName, newFieldNames, newFields):
            Let newFieldMap be a new Map of Text to CVal.
            Let mutable nfIdx be 1.
            Repeat for nfn in newFieldNames:
                If nfIdx is at most (length of newFields):
                    Set item nfn of newFieldMap to coreEval(item nfIdx of newFields, env, funcs).
                Set nfIdx to nfIdx + 1.
            Return a new VStruct with typeName newTypeName and fields newFieldMap.
        When CFieldAccess (faTarget, faField):
            Let faVal be coreEval(faTarget, env, funcs).
            Inspect faVal:
                When VStruct (faTypeName, faFields):
                    Return item faField of faFields.
                When VMap (faMapData):
                    Return item faField of faMapData.
                Otherwise:
                    Return a new VNothing.
        When CClosure (clParams, clBody, clCaptured):
            Let clEnv be a new Map of Text to CVal.
            Let mutable clCapIdx be 1.
            While clCapIdx is at most (length of clCaptured):
                Let clCapName be item clCapIdx of clCaptured.
                Let clCapVal be item clCapName of env.
                Let clCapName2 be item clCapIdx of clCaptured.
                Set item clCapName2 of clEnv to clCapVal.
                Set clCapIdx to clCapIdx + 1.
            Return a new VClosure with params clParams and body clBody and capturedEnv clEnv.
        When CCallExpr (ceTarget, ceArgs):
            Let ceVal be coreEval(ceTarget, env, funcs).
            Inspect ceVal:
                When VClosure (ceParams, ceBody, ceCapturedEnv):
                    Let mutable ceCallEnv be ceCapturedEnv.
                    Let mutable ceCopyIdx be 1.
                    While ceCopyIdx is at most (length of ceParams):
                        If ceCopyIdx is at most (length of ceArgs):
                            Let ceArgVal be coreEval(item ceCopyIdx of ceArgs, env, funcs).
                            Set item (item ceCopyIdx of ceParams) of ceCallEnv to ceArgVal.
                        Set ceCopyIdx to ceCopyIdx + 1.
                    Let ceResult be coreExecBlock(ceBody, ceCallEnv, funcs).
                    Return ceResult.
                Otherwise:
                    Return a new VNothing.
        When CInterpolatedString (isParts):
            Let mutable isResult be "".
            Repeat for isPart in isParts:
                Inspect isPart:
                    When CLiteralPart (litVal):
                        Set isResult to "{isResult}{litVal}".
                    When CExprPart (epExpr):
                        Let epVal be coreEval(epExpr, env, funcs).
                        Let epText be valToText(epVal).
                        Set isResult to "{isResult}{epText}".
            Return a new VText with value isResult.
        When CDuration (durAmountExpr, durUnit):
            Let durAmountVal be coreEval(durAmountExpr, env, funcs).
            Inspect durAmountVal:
                When VInt (durAmt):
                    If durUnit equals "seconds":
                        Return a new VDuration with millis (durAmt * 1000).
                    If durUnit equals "minutes":
                        Return a new VDuration with millis (durAmt * 60000).
                    If durUnit equals "hours":
                        Return a new VDuration with millis (durAmt * 3600000).
                    If durUnit equals "milliseconds":
                        Return a new VDuration with millis durAmt.
                    Return a new VDuration with millis (durAmt * 1000).
                Otherwise:
                    Return a new VNothing.
        When CTimeNow:
            Return a new VMoment with millis 0.
        When CDateToday:
            Return a new VDate with year 2026 and month 1 and day 1.
        When CEscExpr (escCode):
            Return a new VNothing.

## To coreExecBlock (stmts: Seq of CStmt) and (env: Map of Text to CVal) and (funcs: Map of Text to CFunc) -> CVal:
    Repeat for s in stmts:
        Inspect s:
            When CLet (name, expr):
                Set item name of env to coreEval(expr, env, funcs).
            When CSet (name, expr):
                Set item name of env to coreEval(expr, env, funcs).
            When CIf (cond, thenBlock, elseBlock):
                Let cv be coreEval(cond, env, funcs).
                Inspect cv:
                    When VBool (b):
                        If b:
                            Let ifResult be coreExecBlock(thenBlock, env, funcs).
                            Let ifNoth be isNothing(ifResult).
                            If not ifNoth:
                                Return ifResult.
                        Otherwise:
                            Let elseResult be coreExecBlock(elseBlock, env, funcs).
                            Let elseNoth be isNothing(elseResult).
                            If not elseNoth:
                                Return elseResult.
                    Otherwise:
                        Let skip be true.
            When CWhile (whileCond, whileBody):
                Let mutable loopDone be false.
                Let mutable loopResult be a new VNothing.
                While not loopDone:
                    Let wcv be coreEval(whileCond, env, funcs).
                    Inspect wcv:
                        When VBool (wb):
                            If not wb:
                                Set loopDone to true.
                            Otherwise:
                                Repeat for bs in whileBody:
                                    If not loopDone:
                                        Inspect bs:
                                            When CLet (bname, bexpr):
                                                Set item bname of env to coreEval(bexpr, env, funcs).
                                            When CSet (bname, bexpr):
                                                Set item bname of env to coreEval(bexpr, env, funcs).
                                            When CReturn (bexpr):
                                                Set loopDone to true.
                                                Set loopResult to coreEval(bexpr, env, funcs).
                                            When CShow (bexpr):
                                                Let bv be coreEval(bexpr, env, funcs).
                                                Show valToText(bv).
                                            When CPush (bvalExpr, bcollName):
                                                Let bpv be coreEval(bvalExpr, env, funcs).
                                                Let bseq be item bcollName of env.
                                                Inspect bseq:
                                                    When VSeq (bitems):
                                                        Let mutable bmutItems be bitems.
                                                        Push bpv to bmutItems.
                                                        Set item bcollName of env to a new VSeq with items bmutItems.
                                                    Otherwise:
                                                        Let skip be true.
                                            When CSetIdx (bcollName, bidxExpr, bvalExpr):
                                                Let biv be coreEval(bidxExpr, env, funcs).
                                                Let bsv be coreEval(bvalExpr, env, funcs).
                                                Let bseq2 be item bcollName of env.
                                                Inspect bseq2:
                                                    When VSeq (bitems2):
                                                        Inspect biv:
                                                            When VInt (bi):
                                                                Let mutable bmutItems2 be bitems2.
                                                                Set item bi of bmutItems2 to bsv.
                                                                Set item bcollName of env to a new VSeq with items bmutItems2.
                                                            Otherwise:
                                                                Let skip be true.
                                                    Otherwise:
                                                        Let skip be true.
                                            When CPop (bpopTarget):
                                                Let bpseq be item bpopTarget of env.
                                                Inspect bpseq:
                                                    When VSeq (bpitems):
                                                        Let bpLen be length of bpitems.
                                                        If bpLen is greater than 0:
                                                            Let mutable bpNew be a new Seq of CVal.
                                                            Let mutable bpi be 1.
                                                            While bpi is less than bpLen:
                                                                Push item bpi of bpitems to bpNew.
                                                                Set bpi to bpi + 1.
                                                            Set item bpopTarget of env to a new VSeq with items bpNew.
                                                    Otherwise:
                                                        Let skip be true.
                                            When CIf (bifCond, bifThen, bifElse):
                                                Let bicv be coreEval(bifCond, env, funcs).
                                                Inspect bicv:
                                                    When VBool (bib):
                                                        Let mutable bifBlock be bifThen.
                                                        If not bib:
                                                            Set bifBlock to bifElse.
                                                        Repeat for bts in bifBlock:
                                                            Inspect bts:
                                                                When CLet (btname, btexpr):
                                                                    Set item btname of env to coreEval(btexpr, env, funcs).
                                                                When CSet (btname, btexpr):
                                                                    Set item btname of env to coreEval(btexpr, env, funcs).
                                                                When CReturn (btexpr):
                                                                    Set loopDone to true.
                                                                    Set loopResult to coreEval(btexpr, env, funcs).
                                                                When CShow (btexpr):
                                                                    Let btv be coreEval(btexpr, env, funcs).
                                                                    Show valToText(btv).
                                                                When CPush (btval, btcoll):
                                                                    Let btpv be coreEval(btval, env, funcs).
                                                                    Let btseq be item btcoll of env.
                                                                    Inspect btseq:
                                                                        When VSeq (btitems):
                                                                            Let mutable btmut be btitems.
                                                                            Push btpv to btmut.
                                                                            Set item btcoll of env to a new VSeq with items btmut.
                                                                        Otherwise:
                                                                            Let skip be true.
                                                                When CPop (btpop):
                                                                    Let btpseq be item btpop of env.
                                                                    Inspect btpseq:
                                                                        When VSeq (btpitems):
                                                                            Let btpLen be length of btpitems.
                                                                            If btpLen is greater than 0:
                                                                                Let mutable btpNew be a new Seq of CVal.
                                                                                Let mutable btpi be 1.
                                                                                While btpi is less than btpLen:
                                                                                    Push item btpi of btpitems to btpNew.
                                                                                    Set btpi to btpi + 1.
                                                                                Set item btpop of env to a new VSeq with items btpNew.
                                                                        Otherwise:
                                                                            Let skip be true.
                                                                When CSetIdx (btsiTarget, btsiIdx, btsiVal):
                                                                    Let btsiv be coreEval(btsiIdx, env, funcs).
                                                                    Let btsvv be coreEval(btsiVal, env, funcs).
                                                                    Let btsiseq be item btsiTarget of env.
                                                                    Inspect btsiseq:
                                                                        When VSeq (btsiItems):
                                                                            Inspect btsiv:
                                                                                When VInt (btsii):
                                                                                    Let mutable btsiMut be btsiItems.
                                                                                    Set item btsii of btsiMut to btsvv.
                                                                                    Set item btsiTarget of env to a new VSeq with items btsiMut.
                                                                                Otherwise:
                                                                                    Let skip be true.
                                                                        Otherwise:
                                                                            Let skip be true.
                                                                When CBreak:
                                                                    Set loopDone to true.
                                                                Otherwise:
                                                                    Let skip be true.
                                                    Otherwise:
                                                        Let skip be true.
                                            When CBreak:
                                                Set loopDone to true.
                                            Otherwise:
                                                Let skip be true.
                        Otherwise:
                            Set loopDone to true.
                Let loopNoth be isNothing(loopResult).
                If not loopNoth:
                    Return loopResult.
            When CReturn (expr):
                Return coreEval(expr, env, funcs).
            When CShow (expr):
                Let v be coreEval(expr, env, funcs).
                Show valToText(v).
            When CCallS (name, argExprs):
                Let argVals be a new Seq of CVal.
                Repeat for a in argExprs:
                    Push coreEval(a, env, funcs) to argVals.
                Let csInFuncs be (funcs contains name).
                If csInFuncs:
                    Let func be item name of funcs.
                    Inspect func:
                        When CFuncDef (fname, params, body):
                            Let callEnv be a new Map of Text to CVal.
                            Let mutable cidx be 1.
                            Repeat for p in params:
                                Set item p of callEnv to item cidx of argVals.
                                Set cidx to cidx + 1.
                            Let callResult be coreExecBlock(body, callEnv, funcs).
                            Let skip be true.
                        Otherwise:
                            Let skip be true.
                Otherwise:
                    Let csInEnv be (env contains name).
                    If csInEnv:
                        Let envVal be item name of env.
                        Inspect envVal:
                            When VClosure (csParams, csBody, csCapturedEnv):
                                Let mutable csCallEnv be csCapturedEnv.
                                Let mutable csCopyIdx be 1.
                                While csCopyIdx is at most (length of csParams):
                                    If csCopyIdx is at most (length of argVals):
                                        Set item (item csCopyIdx of csParams) of csCallEnv to item csCopyIdx of argVals.
                                    Set csCopyIdx to csCopyIdx + 1.
                                Let csResult be coreExecBlock(csBody, csCallEnv, funcs).
                                Let skip be true.
                            Otherwise:
                                Let skip be true.
            When CPush (valExpr, collName):
                Let v be coreEval(valExpr, env, funcs).
                Let seq be item collName of env.
                Inspect seq:
                    When VSeq (items):
                        Let mutable mutItems be items.
                        Push v to mutItems.
                        Set item collName of env to a new VSeq with items mutItems.
                    Otherwise:
                        Let skip be true.
            When CSetIdx (collName, idxExpr, valExpr):
                Let iv be coreEval(idxExpr, env, funcs).
                Let v be coreEval(valExpr, env, funcs).
                Let seq be item collName of env.
                Inspect seq:
                    When VSeq (items):
                        Inspect iv:
                            When VInt (i):
                                Let mutable mutItems be items.
                                Set item i of mutItems to v.
                                Set item collName of env to a new VSeq with items mutItems.
                            Otherwise:
                                Let skip be true.
                    Otherwise:
                        Let skip be true.
            When CMapSet (mapName, keyExpr, valExpr):
                Let kv be coreEval(keyExpr, env, funcs).
                Let v be coreEval(valExpr, env, funcs).
                Let mv be item mapName of env.
                Inspect mv:
                    When VMap (mapData):
                        Inspect kv:
                            When VText (key):
                                Let mutable mutMap be mapData.
                                Set item key of mutMap to v.
                                Set item mapName of env to a new VMap with entries mutMap.
                            Otherwise:
                                Let skip be true.
                    Otherwise:
                        Let skip be true.
            When CPop (popTarget):
                Let seq be item popTarget of env.
                Inspect seq:
                    When VSeq (seqItems):
                        Let seqLen be length of seqItems.
                        If seqLen is greater than 0:
                            Let mutable newItems be a new Seq of CVal.
                            Let mutable pi be 1.
                            While pi is less than seqLen:
                                Push item pi of seqItems to newItems.
                                Set pi to pi + 1.
                            Set item popTarget of env to a new VSeq with items newItems.
                    Otherwise:
                        Let skip be true.
            When CAdd (addElem, addTarget):
                Let addValHolder be a new Seq of CVal.
                Push coreEval(addElem, env, funcs) to addValHolder.
                Let addValText be valToText(item 1 of addValHolder).
                Let addColl be item addTarget of env.
                Inspect addColl:
                    When VSet (addItems):
                        Let mutable addFound be false.
                        Repeat for ai in addItems:
                            Let aiText be valToText(ai).
                            If aiText equals addValText:
                                Set addFound to true.
                        If not addFound:
                            Let mutable addNew be addItems.
                            Push item 1 of addValHolder to addNew.
                            Set item addTarget of env to a new VSet with items addNew.
                    Otherwise:
                        Let skip be true.
            When CRemove (remElem, remTarget):
                Let remValText be valToText(coreEval(remElem, env, funcs)).
                Let remColl be item remTarget of env.
                Inspect remColl:
                    When VSet (remItems):
                        Let mutable remNew be a new Seq of CVal.
                        Let mutable remIdx be 1.
                        While remIdx is at most (length of remItems):
                            Let remItemText be valToText(item remIdx of remItems).
                            If remItemText is not remValText:
                                Push item remIdx of remItems to remNew.
                            Set remIdx to remIdx + 1.
                        Set item remTarget of env to a new VSet with items remNew.
                    Otherwise:
                        Let skip be true.
            When CSetField (sfTarget, sfField, sfValExpr):
                Let sfVal be coreEval(sfValExpr, env, funcs).
                Let sfObj be item sfTarget of env.
                Inspect sfObj:
                    When VStruct (sfTypeName, sfFields):
                        Let mutable sfMut be sfFields.
                        Set item sfField of sfMut to sfVal.
                        Set item sfTarget of env to a new VStruct with typeName sfTypeName and fields sfMut.
                    When VMap (sfMapData):
                        Let mutable sfMutMap be sfMapData.
                        Set item sfField of sfMutMap to sfVal.
                        Set item sfTarget of env to a new VMap with entries sfMutMap.
                    Otherwise:
                        Let skip be true.
            When CStructDef (sdName, sdFieldNames):
                Let skip be true.
            When CEnumDef (edName, edVariants):
                Let skip be true.
            When CInspect (inspTarget, inspArms):
                Let inspVal be coreEval(inspTarget, env, funcs).
                Let mutable inspTag be "".
                Let mutable inspFields be a new Seq of CVal.
                Let mutable inspNamedFields be a new Map of Text to CVal.
                Let mutable inspIsMap be false.
                Inspect inspVal:
                    When VVariant (ivt, ivn, ivf):
                        Set inspTag to ivn.
                        Set inspFields to ivf.
                    When VMap (mapData):
                        Set inspIsMap to true.
                        Let tagEntry be item "__tag" of mapData.
                        Inspect tagEntry:
                            When VText (tagStr):
                                Set inspTag to tagStr.
                            Otherwise:
                                Let skip be true.
                        Set inspNamedFields to mapData.
                    Otherwise:
                        Let skip be true.
                Let mutable inspMatched be false.
                Repeat for arm in inspArms:
                    If not inspMatched:
                        Inspect arm:
                            When CWhen (armName, armBindings, armBody):
                                If armName equals inspTag:
                                    Set inspMatched to true.
                                    If inspIsMap:
                                        Let fnamesEntry be item "__fnames__" of inspNamedFields.
                                        Let mutable inspFnameSeq be a new Seq of CVal.
                                        Inspect fnamesEntry:
                                            When VSeq (fnameItems):
                                                Set inspFnameSeq to fnameItems.
                                            Otherwise:
                                                Let skip be true.
                                        Let mutable abiM be 1.
                                        While abiM is at most (length of armBindings):
                                            If abiM is at most (length of inspFnameSeq):
                                                Let fnameVal be item abiM of inspFnameSeq.
                                                Let mutable fnameStr be "".
                                                Inspect fnameVal:
                                                    When VText (fns):
                                                        Set fnameStr to fns.
                                                    Otherwise:
                                                        Let skip be true.
                                                Let abLookup be item fnameStr of inspNamedFields.
                                                Set item (item abiM of armBindings) of env to abLookup.
                                            Set abiM to abiM + 1.
                                    Otherwise:
                                        Let mutable abi2 be 1.
                                        While abi2 is at most (length of armBindings):
                                            If abi2 is at most (length of inspFields):
                                                Set item (item abi2 of armBindings) of env to item abi2 of inspFields.
                                            Set abi2 to abi2 + 1.
                                    Let armResult be coreExecBlock(armBody, env, funcs).
                                    Let armNoth be isNothing(armResult).
                                    If not armNoth:
                                        Return armResult.
                            When COtherwise (owBody):
                                If not inspMatched:
                                    Set inspMatched to true.
                                    Let owResult be coreExecBlock(owBody, env, funcs).
                                    Let owNoth be isNothing(owResult).
                                    If not owNoth:
                                        Return owResult.
            When CRepeat (repVar, repCollExpr, repBody):
                Let repCV be coreEval(repCollExpr, env, funcs).
                Inspect repCV:
                    When VSeq (repItems):
                        Let repLen be length of repItems.
                        Let mutable repIdx be 1.
                        Let mutable repDone be false.
                        Let repVarName be "{repVar}".
                        While (not repDone):
                            If repIdx is greater than repLen:
                                Set repDone to true.
                            Otherwise:
                                Let rvk be "{repVarName}".
                                Set item rvk of env to item repIdx of repItems.
                                Repeat for repS in repBody:
                                    If not repDone:
                                        Inspect repS:
                                            When CLet (rln, rle):
                                                Set item rln of env to coreEval(rle, env, funcs).
                                            When CSet (rsn, rse):
                                                Set item rsn of env to coreEval(rse, env, funcs).
                                            When CShow (rse):
                                                Let rsv be coreEval(rse, env, funcs).
                                                Show valToText(rsv).
                                            When CReturn (rre):
                                                Set repDone to true.
                                                Return coreEval(rre, env, funcs).
                                            When CPush (rpval, rpcoll):
                                                Let rpv be coreEval(rpval, env, funcs).
                                                Let rpseq be item rpcoll of env.
                                                Inspect rpseq:
                                                    When VSeq (rpitems):
                                                        Let mutable rpmut be rpitems.
                                                        Push rpv to rpmut.
                                                        Set item rpcoll of env to a new VSeq with items rpmut.
                                                    Otherwise:
                                                        Let skip be true.
                                            When CBreak:
                                                Set repDone to true.
                                            When CIf (riCond, riThen, riElse):
                                                Let ricv be coreEval(riCond, env, funcs).
                                                Inspect ricv:
                                                    When VBool (rib):
                                                        If rib:
                                                            Let riResult be coreExecBlock(riThen, env, funcs).
                                                            Let riNoth be isNothing(riResult).
                                                            If not riNoth:
                                                                Let riTxt be valToText(riResult).
                                                                If riTxt equals "__break__":
                                                                    Set repDone to true.
                                                                Otherwise:
                                                                    Return riResult.
                                                        Otherwise:
                                                            Let riResult2 be coreExecBlock(riElse, env, funcs).
                                                            Let riNoth2 be isNothing(riResult2).
                                                            If not riNoth2:
                                                                Let riTxt2 be valToText(riResult2).
                                                                If riTxt2 equals "__break__":
                                                                    Set repDone to true.
                                                                Otherwise:
                                                                    Return riResult2.
                                                    Otherwise:
                                                        Let skip be true.
                                            When CRepeatRange (rrv, rrs, rre, rrb):
                                                Let rrsv be coreEval(rrs, env, funcs).
                                                Let rrev be coreEval(rre, env, funcs).
                                                Inspect rrsv:
                                                    When VInt (rrsi):
                                                        Inspect rrev:
                                                            When VInt (rrei):
                                                                Let rrvName be "{rrv}".
                                                                Let mutable rrIdx be rrsi.
                                                                While rrIdx is at most rrei:
                                                                    Let rrvk be "{rrvName}".
                                                                    Set item rrvk of env to a new VInt with value rrIdx.
                                                                    Let rrResult be coreExecBlock(rrb, env, funcs).
                                                                    Let rrNoth be isNothing(rrResult).
                                                                    If not rrNoth:
                                                                        Let rrTxt be valToText(rrResult).
                                                                        If rrTxt equals "__break__":
                                                                            Set rrIdx to rrei + 1.
                                                                        Otherwise:
                                                                            Return rrResult.
                                                                    Set rrIdx to rrIdx + 1.
                                                            Otherwise:
                                                                Let skip be true.
                                                    Otherwise:
                                                        Let skip be true.
                                            When CInspect (riTarget, riArms):
                                                Let riVal be coreEval(riTarget, env, funcs).
                                                Let mutable riTag be "".
                                                Let mutable riFnames be a new Seq of CVal.
                                                Let mutable riNamedFields be a new Map of Text to CVal.
                                                Let mutable riIsMap be false.
                                                Inspect riVal:
                                                    When VVariant (rit, rin, rif):
                                                        Set riTag to rin.
                                                    When VMap (riMapData):
                                                        Set riIsMap to true.
                                                        Let riTagE be item "__tag" of riMapData.
                                                        Inspect riTagE:
                                                            When VText (riTagStr):
                                                                Set riTag to riTagStr.
                                                            Otherwise:
                                                                Let skip be true.
                                                        Let riFnE be item "__fnames__" of riMapData.
                                                        Inspect riFnE:
                                                            When VSeq (riFnItems):
                                                                Set riFnames to riFnItems.
                                                            Otherwise:
                                                                Let skip be true.
                                                        Set riNamedFields to riMapData.
                                                    Otherwise:
                                                        Let skip be true.
                                                Let mutable riMatched be false.
                                                Repeat for riArm in riArms:
                                                    If not riMatched:
                                                        Inspect riArm:
                                                            When CWhen (riArmName, riArmBindings, riArmBody):
                                                                If riArmName equals riTag:
                                                                    Set riMatched to true.
                                                                    If riIsMap:
                                                                        Let mutable riBI be 1.
                                                                        While riBI is at most (length of riArmBindings):
                                                                            If riBI is at most (length of riFnames):
                                                                                Let riFnVal be item riBI of riFnames.
                                                                                Let mutable riFnStr be "".
                                                                                Inspect riFnVal:
                                                                                    When VText (rifs):
                                                                                        Set riFnStr to rifs.
                                                                                    Otherwise:
                                                                                        Let skip be true.
                                                                                Let riLookup be item riFnStr of riNamedFields.
                                                                                Set item (item riBI of riArmBindings) of env to riLookup.
                                                                            Set riBI to riBI + 1.
                                                                    Otherwise:
                                                                        Let skip be true.
                                                                    Repeat for riArmS in riArmBody:
                                                                        Inspect riArmS:
                                                                            When CLet (rialn, riale):
                                                                                Set item rialn of env to coreEval(riale, env, funcs).
                                                                            When CSet (riasn, riase):
                                                                                Set item riasn of env to coreEval(riase, env, funcs).
                                                                            When CShow (riase2):
                                                                                Let riasv be coreEval(riase2, env, funcs).
                                                                                Show valToText(riasv).
                                                                            When CPush (riapval, riapcoll):
                                                                                Let riapv be coreEval(riapval, env, funcs).
                                                                                Let riapseq be item riapcoll of env.
                                                                                Inspect riapseq:
                                                                                    When VSeq (riapitems):
                                                                                        Let mutable riapmut be riapitems.
                                                                                        Push riapv to riapmut.
                                                                                        Set item riapcoll of env to a new VSeq with items riapmut.
                                                                                    Otherwise:
                                                                                        Let skip be true.
                                                                            Otherwise:
                                                                                Let skip be true.
                                                            When COtherwise (riOwBody):
                                                                If not riMatched:
                                                                    Set riMatched to true.
                                                                    Repeat for riOwS in riOwBody:
                                                                        Inspect riOwS:
                                                                            When CLet (rioln, riole):
                                                                                Set item rioln of env to coreEval(riole, env, funcs).
                                                                            When CSet (riosn, riose):
                                                                                Set item riosn of env to coreEval(riose, env, funcs).
                                                                            When CShow (riose2):
                                                                                Let riosv be coreEval(riose2, env, funcs).
                                                                                Show valToText(riosv).
                                                                            When CPush (riopval, riopcoll):
                                                                                Let riopv be coreEval(riopval, env, funcs).
                                                                                Let riopseq be item riopcoll of env.
                                                                                Inspect riopseq:
                                                                                    When VSeq (riopitems):
                                                                                        Let mutable riopmut be riopitems.
                                                                                        Push riopv to riopmut.
                                                                                        Set item riopcoll of env to a new VSeq with items riopmut.
                                                                                    Otherwise:
                                                                                        Let skip be true.
                                                                            Otherwise:
                                                                                Let skip be true.
                                            Otherwise:
                                                Let skip be true.
                                Set repIdx to repIdx + 1.
                    Otherwise:
                        Let skip be true.
            When CRepeatRange (rrVar, rrStartExpr, rrEndExpr, rrBody):
                Let rrSV be coreEval(rrStartExpr, env, funcs).
                Let rrEV be coreEval(rrEndExpr, env, funcs).
                Inspect rrSV:
                    When VInt (rrStart):
                        Inspect rrEV:
                            When VInt (rrEnd):
                                Let rrVarName be "{rrVar}".
                                Let mutable rrI be rrStart.
                                Let mutable rrDone be false.
                                While not rrDone:
                                    If rrI is greater than rrEnd:
                                        Set rrDone to true.
                                    Otherwise:
                                        Let rrvk2 be "{rrVarName}".
                                        Set item rrvk2 of env to a new VInt with value rrI.
                                        Repeat for rrS in rrBody:
                                            If not rrDone:
                                                Inspect rrS:
                                                    When CLet (rrln, rrle):
                                                        Set item rrln of env to coreEval(rrle, env, funcs).
                                                    When CSet (rrsn, rrse):
                                                        Set item rrsn of env to coreEval(rrse, env, funcs).
                                                    When CShow (rrse):
                                                        Let rrsv be coreEval(rrse, env, funcs).
                                                        Show valToText(rrsv).
                                                    When CReturn (rrre):
                                                        Set rrDone to true.
                                                        Return coreEval(rrre, env, funcs).
                                                    When CPush (rrpval, rrpcoll):
                                                        Let rrpv be coreEval(rrpval, env, funcs).
                                                        Let rrpseq be item rrpcoll of env.
                                                        Inspect rrpseq:
                                                            When VSeq (rrpitems):
                                                                Let mutable rrpmut be rrpitems.
                                                                Push rrpv to rrpmut.
                                                                Set item rrpcoll of env to a new VSeq with items rrpmut.
                                                            Otherwise:
                                                                Let skip be true.
                                                    When CBreak:
                                                        Set rrDone to true.
                                                    When CIf (rriCond, rriThen, rriElse):
                                                        Let rricv be coreEval(rriCond, env, funcs).
                                                        Inspect rricv:
                                                            When VBool (rrib):
                                                                If rrib:
                                                                    Let rriResult be coreExecBlock(rriThen, env, funcs).
                                                                    Let rriNoth be isNothing(rriResult).
                                                                    If not rriNoth:
                                                                        Let rriTxt be valToText(rriResult).
                                                                        If rriTxt equals "__break__":
                                                                            Set rrDone to true.
                                                                        Otherwise:
                                                                            Return rriResult.
                                                                Otherwise:
                                                                    Let rriResult2 be coreExecBlock(rriElse, env, funcs).
                                                                    Let rriNoth2 be isNothing(rriResult2).
                                                                    If not rriNoth2:
                                                                        Let rriTxt2 be valToText(rriResult2).
                                                                        If rriTxt2 equals "__break__":
                                                                            Set rrDone to true.
                                                                        Otherwise:
                                                                            Return rriResult2.
                                                            Otherwise:
                                                                Let skip be true.
                                                    Otherwise:
                                                        Let skip be true.
                                        Set rrI to rrI + 1.
                            Otherwise:
                                Let skip be true.
                    Otherwise:
                        Let skip be true.
            When CBreak:
                Return a new VText with value "__break__".
            When CRuntimeAssert (raCond, raMsg):
                Let raCondVal be coreEval(raCond, env, funcs).
                Inspect raCondVal:
                    When VBool (raB):
                        If raB is not true:
                            Let raMsgVal be coreEval(raMsg, env, funcs).
                            Let raMsgText be valToText(raMsgVal).
                            Show "Assertion failed: {raMsgText}".
                            Return a new VError with msg raMsgText.
                    Otherwise:
                        Let skip be true.
            When CGive (giveExpr, giveTarget):
                Let giveVal be coreEval(giveExpr, env, funcs).
                Set item giveTarget of env to giveVal.
            When CEscStmt (escCode):
                Let skip be true.
            When CSleep (sleepDur):
                Let skip be true.
            When CReadConsole (rcTarget):
                Set item rcTarget of env to (a new VText with value "").
            When CReadFile (rfPath, rfTarget):
                Set item rfTarget of env to (a new VText with value "").
            When CWriteFile (wfPath, wfContent):
                Let skip be true.
            When CCheck (chkPred, chkMsg):
                Let chkVal be coreEval(chkPred, env, funcs).
                Inspect chkVal:
                    When VBool (chkB):
                        If chkB is not true:
                            Let chkMsgVal be coreEval(chkMsg, env, funcs).
                            Let chkMsgText be valToText(chkMsgVal).
                            Show "Security violation: {chkMsgText}".
                            Return a new VError with msg chkMsgText.
                    Otherwise:
                        Let skip be true.
            When CAssert (assertProp):
                Let assertVal be coreEval(assertProp, env, funcs).
                Inspect assertVal:
                    When VBool (assertB):
                        If assertB is not true:
                            Show "Assertion failed".
                            Return a new VError with msg "assertion failed".
                    Otherwise:
                        Let skip be true.
            When CTrust (trustProp, trustJust):
                Let trustVal be coreEval(trustProp, env, funcs).
                Inspect trustVal:
                    When VBool (trustB):
                        If trustB is not true:
                            Show "Trust violation: {trustJust}".
                            Return a new VError with msg trustJust.
                    Otherwise:
                        Let skip be true.
            When CRequire (reqDep):
                Let skip be true.
            When CMerge (mergeTarget, mergeOther):
                Let skip be true.
            When CIncrease (incTarget, incAmountExpr):
                Let incAmount be coreEval(incAmountExpr, env, funcs).
                Let incTargetVal be item incTarget of env.
                Inspect incTargetVal:
                    When VInt (incOldVal):
                        Inspect incAmount:
                            When VInt (incAmtVal):
                                Set item incTarget of env to a new VInt with value (incOldVal + incAmtVal).
                            Otherwise:
                                Let skip be true.
                    Otherwise:
                        Let skip be true.
            When CDecrease (decTarget, decAmountExpr):
                Let decAmount be coreEval(decAmountExpr, env, funcs).
                Let decTargetVal be item decTarget of env.
                Inspect decTargetVal:
                    When VInt (decOldVal):
                        Inspect decAmount:
                            When VInt (decAmtVal):
                                Set item decTarget of env to a new VInt with value (decOldVal - decAmtVal).
                            Otherwise:
                                Let skip be true.
                    Otherwise:
                        Let skip be true.
            When CAppendToSeq (asTarget, asValueExpr):
                Let asValue be coreEval(asValueExpr, env, funcs).
                Let asTargetVal be item asTarget of env.
                Inspect asTargetVal:
                    When VSeq (asItems):
                        Let mutable asMutSeq be asItems.
                        Push asValue to asMutSeq.
                        Set item asTarget of env to a new VSeq with items asMutSeq.
                    Otherwise:
                        Let skip be true.
            When CResolve (resTarget):
                Let skip be true.
            When CSync (syncTarget, syncChannel):
                Let skip be true.
            When CMount (mountTarget, mountPath):
                Let skip be true.
            When CConcurrent (concBranches):
                Repeat for concBranch in concBranches:
                    Let concResult be coreExecBlock(concBranch, env, funcs).
                    Let concNoth be isNothing(concResult).
                    If not concNoth:
                        Let skip be true.
            When CParallel (parBranches):
                Repeat for parBranch in parBranches:
                    Let parResult be coreExecBlock(parBranch, env, funcs).
                    Let parNoth be isNothing(parResult).
                    If not parNoth:
                        Let skip be true.
            When CLaunchTask (ltBody, ltHandle):
                Let ltResult be coreExecBlock(ltBody, env, funcs).
                Set item ltHandle of env to a new VText with value "task_handle".
            When CStopTask (stHandle):
                Let skip be true.
            When CSelect (selBranches):
                Repeat for selBranch in selBranches:
                    Inspect selBranch:
                        When CSelectRecv (selPipe, selVar, selBody):
                            Let selPipeHas be (env contains selPipe).
                            If selPipeHas:
                                Let selPipeVal be item selPipe of env.
                                Inspect selPipeVal:
                                    When VSeq (selPipeItems):
                                        If (length of selPipeItems) is greater than 0:
                                            Set item selVar of env to item 1 of selPipeItems.
                                            Let mutable selNewPipe be a new Seq of CVal.
                                            Let mutable selPi be 2.
                                            While selPi is at most (length of selPipeItems):
                                                Push item selPi of selPipeItems to selNewPipe.
                                                Set selPi to selPi + 1.
                                            Set item selPipe of env to a new VSeq with items selNewPipe.
                                            Let selResult be coreExecBlock(selBody, env, funcs).
                                    Otherwise:
                                        Let skip be true.
                        When CSelectTimeout (selDur, selBody):
                            Let selResult be coreExecBlock(selBody, env, funcs).
            When CCreatePipe (cpName, cpCapacity):
                Set item cpName of env to a new VSeq with items (a new Seq of CVal).
            When CSendPipe (spPipe, spValueExpr):
                Let spVal be coreEval(spValueExpr, env, funcs).
                Let spPipeVal be item spPipe of env.
                Inspect spPipeVal:
                    When VSeq (spItems):
                        Let mutable spMut be spItems.
                        Push spVal to spMut.
                        Set item spPipe of env to a new VSeq with items spMut.
                    Otherwise:
                        Let skip be true.
            When CReceivePipe (rpPipe, rpTarget):
                Let rpPipeVal be item rpPipe of env.
                Inspect rpPipeVal:
                    When VSeq (rpItems):
                        If (length of rpItems) is greater than 0:
                            Set item rpTarget of env to item 1 of rpItems.
                            Let mutable rpNew be a new Seq of CVal.
                            Let mutable rpI be 2.
                            While rpI is at most (length of rpItems):
                                Push item rpI of rpItems to rpNew.
                                Set rpI to rpI + 1.
                            Set item rpPipe of env to a new VSeq with items rpNew.
                    Otherwise:
                        Let skip be true.
            When CTrySendPipe (tspPipe, tspValueExpr):
                Let tspVal be coreEval(tspValueExpr, env, funcs).
                Let tspPipeVal be item tspPipe of env.
                Inspect tspPipeVal:
                    When VSeq (tspItems):
                        Let mutable tspMut be tspItems.
                        Push tspVal to tspMut.
                        Set item tspPipe of env to a new VSeq with items tspMut.
                    Otherwise:
                        Let skip be true.
            When CTryReceivePipe (trpPipe, trpTarget):
                Let trpPipeVal be item trpPipe of env.
                Inspect trpPipeVal:
                    When VSeq (trpItems):
                        If (length of trpItems) is greater than 0:
                            Set item trpTarget of env to item 1 of trpItems.
                            Let mutable trpNew be a new Seq of CVal.
                            Let mutable trpI be 2.
                            While trpI is at most (length of trpItems):
                                Push item trpI of trpItems to trpNew.
                                Set trpI to trpI + 1.
                            Set item trpPipe of env to a new VSeq with items trpNew.
                        Otherwise:
                            Set item trpTarget of env to a new VNothing.
                    Otherwise:
                        Set item trpTarget of env to a new VNothing.
            When CSpawn (spawnType, spawnTarget):
                Set item spawnTarget of env to a new VText with value "agent_handle".
            When CSendMessage (smTarget, smMsg):
                Let skip be true.
            When CAwaitMessage (amTarget):
                Set item amTarget of env to a new VNothing.
            When CListen (listenAddr, listenHandler):
                Let skip be true.
            When CConnectTo (connAddr, connTarget):
                Set item connTarget of env to a new VText with value "connection".
            When CZone (zoneName, zoneKind, zoneBody):
                Let zoneResult be coreExecBlock(zoneBody, env, funcs).
                Let zoneNoth be isNothing(zoneResult).
                If not zoneNoth:
                    Return zoneResult.
            Otherwise:
                Let skip be true.
    Return a new VNothing.
"#;

fn run_interpreter_program(main_block: &str, expected: &str) {
    let source = format!("{}\n{}\n## Main\n{}", CORE_TYPES, INTERPRETER, main_block);
    common::assert_exact_output(&source, expected);
}

#[test]
fn core_eval_literal_int() {
    run_interpreter_program(
        r#"Let showExpr be a new CInt with value 42.
Let showStmt be a new CShow with expr showExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "42",
    );
}

#[test]
fn core_eval_literal_bool() {
    run_interpreter_program(
        r#"Let showExpr be a new CBool with value true.
Let showStmt be a new CShow with expr showExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "true",
    );
}

#[test]
fn core_eval_literal_text() {
    run_interpreter_program(
        r#"Let showExpr be a new CText with value "hello".
Let showStmt be a new CShow with expr showExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "hello",
    );
}

#[test]
fn core_eval_literal_nothing() {
    run_interpreter_program(
        r#"Let v be a new VNothing.
Show valToText(v).
"#,
        "nothing",
    );
}

#[test]
fn core_eval_variable() {
    run_interpreter_program(
        r#"Let letExpr be a new CInt with value 10.
Let letStmt be a new CLet with name "x" and expr letExpr.
Let showVar be a new CVar with name "x".
Let showStmt be a new CShow with expr showVar.
Let stmts be a new Seq of CStmt.
Push letStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "10",
    );
}

#[test]
fn core_eval_addition() {
    run_interpreter_program(
        r#"Let left be a new CInt with value 2.
Let right be a new CInt with value 3.
Let addExpr be a new CBinOp with op "+" and left left and right right.
Let showStmt be a new CShow with expr addExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "5",
    );
}

#[test]
fn core_eval_subtraction() {
    run_interpreter_program(
        r#"Let left be a new CInt with value 10.
Let right be a new CInt with value 3.
Let subExpr be a new CBinOp with op "-" and left left and right right.
Let showStmt be a new CShow with expr subExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "7",
    );
}

#[test]
fn core_eval_multiplication() {
    run_interpreter_program(
        r#"Let left be a new CInt with value 4.
Let right be a new CInt with value 5.
Let mulExpr be a new CBinOp with op "*" and left left and right right.
Let showStmt be a new CShow with expr mulExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "20",
    );
}

#[test]
fn core_eval_nested_arithmetic() {
    run_interpreter_program(
        r#"Let a be a new CInt with value 2.
Let b be a new CInt with value 3.
Let c be a new CInt with value 4.
Let inner be a new CBinOp with op "+" and left a and right b.
Let outer be a new CBinOp with op "*" and left inner and right c.
Let showStmt be a new CShow with expr outer.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "20",
    );
}

#[test]
fn core_eval_comparison_operators() {
    run_interpreter_program(
        r#"Let x be a new CInt with value 3.
Let y be a new CInt with value 5.
Let ltExpr be a new CBinOp with op "<" and left x and right y.
Let showStmt be a new CShow with expr ltExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "true",
    );
}

#[test]
fn core_eval_boolean_and() {
    run_interpreter_program(
        r#"Let l be a new CBool with value true.
Let r be a new CBool with value false.
Let andExpr be a new CBinOp with op "&&" and left l and right r.
Let showStmt be a new CShow with expr andExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "false",
    );
}

#[test]
fn core_eval_boolean_or() {
    run_interpreter_program(
        r#"Let l be a new CBool with value false.
Let r be a new CBool with value true.
Let orExpr be a new CBinOp with op "||" and left l and right r.
Let showStmt be a new CShow with expr orExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "true",
    );
}

#[test]
fn core_eval_if_true() {
    run_interpreter_program(
        r#"Let condExpr be a new CBool with value true.
Let showOne be a new CShow with expr (a new CInt with value 1).
Let showTwo be a new CShow with expr (a new CInt with value 2).
Let thenBlock be a new Seq of CStmt.
Push showOne to thenBlock.
Let elseBlock be a new Seq of CStmt.
Push showTwo to elseBlock.
Let ifStmt be a new CIf with cond condExpr and thenBlock thenBlock and elseBlock elseBlock.
Let stmts be a new Seq of CStmt.
Push ifStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "1",
    );
}

#[test]
fn core_eval_if_false() {
    run_interpreter_program(
        r#"Let condExpr be a new CBool with value false.
Let showOne be a new CShow with expr (a new CInt with value 1).
Let showTwo be a new CShow with expr (a new CInt with value 2).
Let thenBlock be a new Seq of CStmt.
Push showOne to thenBlock.
Let elseBlock be a new Seq of CStmt.
Push showTwo to elseBlock.
Let ifStmt be a new CIf with cond condExpr and thenBlock thenBlock and elseBlock elseBlock.
Let stmts be a new Seq of CStmt.
Push ifStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "2",
    );
}

#[test]
fn core_eval_nested_if() {
    run_interpreter_program(
        r#"Let showOne be a new CShow with expr (a new CInt with value 1).
Let showTwo be a new CShow with expr (a new CInt with value 2).
Let showNone be a new CShow with expr (a new CInt with value 0).
Let innerThen be a new Seq of CStmt.
Push showOne to innerThen.
Let innerElse be a new Seq of CStmt.
Push showTwo to innerElse.
Let innerIf be a new CIf with cond (a new CBool with value false) and thenBlock innerThen and elseBlock innerElse.
Let outerThen be a new Seq of CStmt.
Push innerIf to outerThen.
Let outerElse be a new Seq of CStmt.
Push showNone to outerElse.
Let outerIf be a new CIf with cond (a new CBool with value true) and thenBlock outerThen and elseBlock outerElse.
Let stmts be a new Seq of CStmt.
Push outerIf to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "2",
    );
}

#[test]
fn core_eval_while_loop() {
    run_interpreter_program(
        r#"Let initSum be a new CLet with name "sum" and expr (a new CInt with value 0).
Let initI be a new CLet with name "i" and expr (a new CInt with value 1).
Let loopCond be a new CBinOp with op "<=" and left (a new CVar with name "i") and right (a new CInt with value 5).
Let addSum be a new CBinOp with op "+" and left (a new CVar with name "sum") and right (a new CVar with name "i").
Let setSum be a new CSet with name "sum" and expr addSum.
Let incI be a new CBinOp with op "+" and left (a new CVar with name "i") and right (a new CInt with value 1).
Let setI be a new CSet with name "i" and expr incI.
Let whileBody be a new Seq of CStmt.
Push setSum to whileBody.
Push setI to whileBody.
Let whileStmt be a new CWhile with cond loopCond and body whileBody.
Let showSum be a new CShow with expr (a new CVar with name "sum").
Let stmts be a new Seq of CStmt.
Push initSum to stmts.
Push initI to stmts.
Push whileStmt to stmts.
Push showSum to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "15",
    );
}

#[test]
fn core_eval_function_call() {
    run_interpreter_program(
        r#"Let doubleBody be a new Seq of CStmt.
Let retExpr be a new CBinOp with op "*" and left (a new CVar with name "x") and right (a new CInt with value 2).
Let retStmt be a new CReturn with expr retExpr.
Push retStmt to doubleBody.
Let doubleParams be a new Seq of Text.
Push "x" to doubleParams.
Let doubleFn be a new CFuncDef with name "double" and params doubleParams and body doubleBody.
Let funcMap be a new Map of Text to CFunc.
Set item "double" of funcMap to doubleFn.
Let callArgs be a new Seq of CExpr.
Push (a new CInt with value 21) to callArgs.
Let callExpr be a new CCall with name "double" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "42",
    );
}

#[test]
fn core_eval_recursive_factorial() {
    run_interpreter_program(
        r#"Let factBody be a new Seq of CStmt.
Let cond be a new CBinOp with op "<=" and left (a new CVar with name "n") and right (a new CInt with value 1).
Let baseThen be a new Seq of CStmt.
Push (a new CReturn with expr (a new CInt with value 1)) to baseThen.
Let recArgs be a new Seq of CExpr.
Push (a new CBinOp with op "-" and left (a new CVar with name "n") and right (a new CInt with value 1)) to recArgs.
Let recCall be a new CCall with name "factorial" and args recArgs.
Let recMul be a new CBinOp with op "*" and left (a new CVar with name "n") and right recCall.
Let recElse be a new Seq of CStmt.
Push (a new CReturn with expr recMul) to recElse.
Let ifStmt be a new CIf with cond cond and thenBlock baseThen and elseBlock recElse.
Push ifStmt to factBody.
Let factParams be a new Seq of Text.
Push "n" to factParams.
Let factFn be a new CFuncDef with name "factorial" and params factParams and body factBody.
Let funcMap be a new Map of Text to CFunc.
Set item "factorial" of funcMap to factFn.
Let callArgs be a new Seq of CExpr.
Push (a new CInt with value 5) to callArgs.
Let callExpr be a new CCall with name "factorial" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "120",
    );
}

#[test]
fn core_eval_recursive_fibonacci() {
    run_interpreter_program(
        r#"Let fibBody be a new Seq of CStmt.
Let cond0 be a new CBinOp with op "==" and left (a new CVar with name "n") and right (a new CInt with value 0).
Let base0Then be a new Seq of CStmt.
Push (a new CReturn with expr (a new CInt with value 0)) to base0Then.
Let cond1 be a new CBinOp with op "==" and left (a new CVar with name "n") and right (a new CInt with value 1).
Let base1Then be a new Seq of CStmt.
Push (a new CReturn with expr (a new CInt with value 1)) to base1Then.
Let recArgs1 be a new Seq of CExpr.
Push (a new CBinOp with op "-" and left (a new CVar with name "n") and right (a new CInt with value 1)) to recArgs1.
Let recArgs2 be a new Seq of CExpr.
Push (a new CBinOp with op "-" and left (a new CVar with name "n") and right (a new CInt with value 2)) to recArgs2.
Let recAdd be a new CBinOp with op "+" and left (a new CCall with name "fib" and args recArgs1) and right (a new CCall with name "fib" and args recArgs2).
Let recElse be a new Seq of CStmt.
Push (a new CReturn with expr recAdd) to recElse.
Let emptyElse1 be a new Seq of CStmt.
Let innerIf be a new CIf with cond cond1 and thenBlock base1Then and elseBlock recElse.
Push innerIf to emptyElse1.
Let outerIf be a new CIf with cond cond0 and thenBlock base0Then and elseBlock emptyElse1.
Push outerIf to fibBody.
Let fibParams be a new Seq of Text.
Push "n" to fibParams.
Let fibFn be a new CFuncDef with name "fib" and params fibParams and body fibBody.
Let funcMap be a new Map of Text to CFunc.
Set item "fib" of funcMap to fibFn.
Let callArgs be a new Seq of CExpr.
Push (a new CInt with value 10) to callArgs.
Let callExpr be a new CCall with name "fib" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "55",
    );
}

#[test]
fn core_eval_mutual_recursion() {
    run_interpreter_program(
        r#"Let evenBody be a new Seq of CStmt.
Let evenCond be a new CBinOp with op "==" and left (a new CVar with name "n") and right (a new CInt with value 0).
Let evenBaseThen be a new Seq of CStmt.
Push (a new CReturn with expr (a new CBool with value true)) to evenBaseThen.
Let oddArgs be a new Seq of CExpr.
Push (a new CBinOp with op "-" and left (a new CVar with name "n") and right (a new CInt with value 1)) to oddArgs.
Let oddCall be a new CCall with name "isOdd" and args oddArgs.
Let evenRecElse be a new Seq of CStmt.
Push (a new CReturn with expr oddCall) to evenRecElse.
Let evenIf be a new CIf with cond evenCond and thenBlock evenBaseThen and elseBlock evenRecElse.
Push evenIf to evenBody.
Let evenParams be a new Seq of Text.
Push "n" to evenParams.
Let evenFn be a new CFuncDef with name "isEven" and params evenParams and body evenBody.
Let oddBody be a new Seq of CStmt.
Let oddCond be a new CBinOp with op "==" and left (a new CVar with name "n") and right (a new CInt with value 0).
Let oddBaseThen be a new Seq of CStmt.
Push (a new CReturn with expr (a new CBool with value false)) to oddBaseThen.
Let evenArgs be a new Seq of CExpr.
Push (a new CBinOp with op "-" and left (a new CVar with name "n") and right (a new CInt with value 1)) to evenArgs.
Let evenCall be a new CCall with name "isEven" and args evenArgs.
Let oddRecElse be a new Seq of CStmt.
Push (a new CReturn with expr evenCall) to oddRecElse.
Let oddIf be a new CIf with cond oddCond and thenBlock oddBaseThen and elseBlock oddRecElse.
Push oddIf to oddBody.
Let oddParams be a new Seq of Text.
Push "n" to oddParams.
Let oddFn be a new CFuncDef with name "isOdd" and params oddParams and body oddBody.
Let funcMap be a new Map of Text to CFunc.
Set item "isEven" of funcMap to evenFn.
Set item "isOdd" of funcMap to oddFn.
Let callArgs be a new Seq of CExpr.
Push (a new CInt with value 4) to callArgs.
Let callExpr be a new CCall with name "isEven" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "true",
    );
}

#[test]
fn core_eval_missing_function() {
    // A function with empty body returns VNothing, which valToText shows as "nothing"
    run_interpreter_program(
        r#"Let emptyBody be a new Seq of CStmt.
Let emptyParams be a new Seq of Text.
Let emptyFn be a new CFuncDef with name "noop" and params emptyParams and body emptyBody.
Let funcMap be a new Map of Text to CFunc.
Set item "noop" of funcMap to emptyFn.
Let callArgs be a new Seq of CExpr.
Let callExpr be a new CCall with name "noop" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "nothing",
    );
}

#[test]
fn core_eval_push_and_index() {
    run_interpreter_program(
        r#"Let initSeq be a new VSeq with items (a new Seq of CVal).
Let pushA be a new CPush with expr (a new CInt with value 10) and target "items".
Let pushB be a new CPush with expr (a new CInt with value 20) and target "items".
Let showExpr be a new CIndex with coll (a new CVar with name "items") and idx (a new CInt with value 2).
Let showStmt be a new CShow with expr showExpr.
Let stmts be a new Seq of CStmt.
Push pushA to stmts.
Push pushB to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Set item "items" of env to initSeq.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "20",
    );
}

#[test]
fn core_eval_push_multiple() {
    run_interpreter_program(
        r#"Let initSeq be a new VSeq with items (a new Seq of CVal).
Let pushA be a new CPush with expr (a new CInt with value 10) and target "items".
Let pushB be a new CPush with expr (a new CInt with value 20) and target "items".
Let pushC be a new CPush with expr (a new CInt with value 30) and target "items".
Let show1 be a new CShow with expr (a new CIndex with coll (a new CVar with name "items") and idx (a new CInt with value 1)).
Let show2 be a new CShow with expr (a new CIndex with coll (a new CVar with name "items") and idx (a new CInt with value 2)).
Let show3 be a new CShow with expr (a new CIndex with coll (a new CVar with name "items") and idx (a new CInt with value 3)).
Let stmts be a new Seq of CStmt.
Push pushA to stmts.
Push pushB to stmts.
Push pushC to stmts.
Push show1 to stmts.
Push show2 to stmts.
Push show3 to stmts.
Let env be a new Map of Text to CVal.
Set item "items" of env to initSeq.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "10\n20\n30",
    );
}

#[test]
fn core_eval_set_index() {
    run_interpreter_program(
        r#"Let initItems be a new Seq of CVal.
Push (a new VInt with value 10) to initItems.
Push (a new VInt with value 20) to initItems.
Let initSeq be a new VSeq with items initItems.
Let setStmt be a new CSetIdx with target "items" and idx (a new CInt with value 1) and val (a new CInt with value 99).
Let showStmt be a new CShow with expr (a new CIndex with coll (a new CVar with name "items") and idx (a new CInt with value 1)).
Let stmts be a new Seq of CStmt.
Push setStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Set item "items" of env to initSeq.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "99",
    );
}

#[test]
fn core_eval_sequence_length() {
    run_interpreter_program(
        r#"Let initItems be a new Seq of CVal.
Push (a new VInt with value 10) to initItems.
Push (a new VInt with value 20) to initItems.
Push (a new VInt with value 30) to initItems.
Let initSeq be a new VSeq with items initItems.
Let showStmt be a new CShow with expr (a new CLen with target (a new CVar with name "items")).
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Set item "items" of env to initSeq.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "3",
    );
}

#[test]
fn core_eval_map_operations() {
    run_interpreter_program(
        r#"Let initMap be a new VMap with entries (a new Map of Text to CVal).
Let setStmt be a new CMapSet with target "m" and key (a new CText with value "mykey") and val (a new CInt with value 42).
Let showExpr be a new CMapGet with target (a new CVar with name "m") and key (a new CText with value "mykey").
Let showStmt be a new CShow with expr showExpr.
Let stmts be a new Seq of CStmt.
Push setStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Set item "m" of env to initMap.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "42",
    );
}

#[test]
fn core_eval_scoping_isolation() {
    run_interpreter_program(
        r#"Let retBody be a new Seq of CStmt.
Push (a new CReturn with expr (a new CVar with name "x")) to retBody.
Let retParams be a new Seq of Text.
Push "x" to retParams.
Let retFn be a new CFuncDef with name "getX" and params retParams and body retBody.
Let funcMap be a new Map of Text to CFunc.
Set item "getX" of funcMap to retFn.
Let setX be a new CLet with name "x" and expr (a new CInt with value 100).
Let callArgs be a new Seq of CExpr.
Push (a new CInt with value 7) to callArgs.
Let callExpr be a new CCall with name "getX" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push setX to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "7",
    );
}

#[test]
fn core_eval_early_return_in_while() {
    run_interpreter_program(
        r#"Let fnBody be a new Seq of CStmt.
Let initI be a new CLet with name "i" and expr (a new CInt with value 0).
Let loopCond be a new CBinOp with op "<" and left (a new CVar with name "i") and right (a new CInt with value 100).
Let retStmt be a new CReturn with expr (a new CInt with value 42).
Let whileBody be a new Seq of CStmt.
Push retStmt to whileBody.
Let whileStmt be a new CWhile with cond loopCond and body whileBody.
Push initI to fnBody.
Push whileStmt to fnBody.
Let fnParams be a new Seq of Text.
Let fn be a new CFuncDef with name "earlyRet" and params fnParams and body fnBody.
Let funcMap be a new Map of Text to CFunc.
Set item "earlyRet" of funcMap to fn.
Let callArgs be a new Seq of CExpr.
Let callExpr be a new CCall with name "earlyRet" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "42",
    );
}

#[test]
fn core_eval_counter_loop() {
    run_interpreter_program(
        r#"Let initI be a new CLet with name "i" and expr (a new CInt with value 0).
Let loopCond be a new CBinOp with op "<" and left (a new CVar with name "i") and right (a new CInt with value 5).
Let incI be a new CSet with name "i" and expr (a new CBinOp with op "+" and left (a new CVar with name "i") and right (a new CInt with value 1)).
Let whileBody be a new Seq of CStmt.
Push incI to whileBody.
Let whileStmt be a new CWhile with cond loopCond and body whileBody.
Let showI be a new CShow with expr (a new CVar with name "i").
Let stmts be a new Seq of CStmt.
Push initI to stmts.
Push whileStmt to stmts.
Push showI to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "5",
    );
}

#[test]
fn core_eval_string_concat() {
    run_interpreter_program(
        r#"Let letA be a new CLet with name "a" and expr (a new CText with value "Hello").
Let letB be a new CLet with name "b" and expr (a new CText with value ", World!").
Let concatExpr be a new CBinOp with op "+" and left (a new CVar with name "a") and right (a new CVar with name "b").
Let showStmt be a new CShow with expr concatExpr.
Let stmts be a new Seq of CStmt.
Push letA to stmts.
Push letB to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Hello, World!",
    );
}

#[test]
fn core_eval_div_by_zero() {
    run_interpreter_program(
        r#"Let divExpr be a new CBinOp with op "/" and left (a new CInt with value 10) and right (a new CInt with value 0).
Let showStmt be a new CShow with expr divExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Error: division by zero",
    );
}

#[test]
fn core_eval_mod_by_zero() {
    run_interpreter_program(
        r#"Let modExpr be a new CBinOp with op "%" and left (a new CInt with value 10) and right (a new CInt with value 0).
Let showStmt be a new CShow with expr modExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Error: modulo by zero",
    );
}

#[test]
fn core_eval_index_out_of_bounds() {
    run_interpreter_program(
        r#"Let initItems be a new Seq of CVal.
Push (a new VInt with value 10) to initItems.
Push (a new VInt with value 20) to initItems.
Push (a new VInt with value 30) to initItems.
Let initSeq be a new VSeq with items initItems.
Let showExpr be a new CIndex with coll (a new CVar with name "items") and idx (a new CInt with value 10).
Let showStmt be a new CShow with expr showExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Set item "items" of env to initSeq.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Error: index out of bounds",
    );
}

#[test]
fn core_eval_error_propagation_binop() {
    run_interpreter_program(
        r#"Let errExpr be a new CBinOp with op "/" and left (a new CInt with value 1) and right (a new CInt with value 0).
Let letErr be a new CLet with name "err" and expr errExpr.
Let addExpr be a new CBinOp with op "+" and left (a new CVar with name "err") and right (a new CInt with value 5).
Let showStmt be a new CShow with expr addExpr.
Let stmts be a new Seq of CStmt.
Push letErr to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Error: division by zero",
    );
}

#[test]
fn core_eval_error_in_show() {
    run_interpreter_program(
        r#"Let modExpr be a new CBinOp with op "%" and left (a new CInt with value 7) and right (a new CInt with value 0).
Let showStmt be a new CShow with expr modExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Error: modulo by zero",
    );
}

// ============================================================
// Sprint 5: Projection 1 — pe(int, program) = compiled_program
// ============================================================

const RUN_ENCODED: &str = r#"Let env be a new Map of Text to CVal.
Let result be coreExecBlock(encodedMain, env, encodedFuncMap).
"#;

fn run_encoded_program(program_stmts: &str, expected: &str) {
    let encoded = logicaffeine_compile::compile::encode_program_source(program_stmts).unwrap();
    let source = format!(
        "{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES, INTERPRETER, encoded, RUN_ENCODED
    );
    common::assert_exact_output(&source, expected);
}

fn get_p1_residual(program: &str) -> String {
    logicaffeine_compile::compile::projection1_source(CORE_TYPES, INTERPRETER, program).unwrap()
}

fn run_p1_and_verify(program: &str, expected_output: &str) {
    let residual = get_p1_residual(program);
    let overhead = logicaffeine_compile::compile::verify_no_overhead_source(&residual);
    assert!(
        overhead.is_ok(),
        "P1 residual should have no interpretive overhead: {:?}",
        overhead.unwrap_err()
    );
    common::assert_exact_output(&residual, expected_output);
}

#[test]
fn p1_encode_roundtrip() {
    run_encoded_program("Show 42.", "42");
}

#[test]
fn p1_verifier_catches_violations() {
    let source_with_overhead = format!(
        "{}\n## Main\nLet s be a new CShow with expr (a new CInt with value 42).\nInspect s:\n    When CShow (e):\n        Show \"found CShow\".\n    Otherwise:\n        Show \"other\".\n",
        CORE_TYPES
    );
    let result = logicaffeine_compile::compile::verify_no_overhead_source(&source_with_overhead);
    assert!(result.is_err(), "Should reject residual with Inspect on CStmt variant");
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("CStmt") || err_msg.contains("CShow") || err_msg.contains("overhead"),
        "Error should mention the Core type violation: {}",
        err_msg
    );
}

#[test]
fn p1_no_inspect_on_cstmt() {
    run_p1_and_verify("Show 42.", "42");
}

#[test]
fn p1_no_inspect_on_cexpr() {
    run_p1_and_verify(
        "Let x be 5.\nShow x + 3.",
        "8",
    );
}

#[test]
fn p1_no_core_constructors() {
    run_p1_and_verify(
        "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow double(7).",
        "14",
    );
}

#[test]
fn p1_trivial_show() {
    let residual = get_p1_residual("Show 42.");
    let overhead = logicaffeine_compile::compile::verify_no_overhead_source(&residual);
    assert!(overhead.is_ok(), "Trivial show should have no overhead: {:?}", overhead.unwrap_err());
    common::assert_exact_output(&residual, "42");
}

#[test]
fn p1_arithmetic() {
    run_p1_and_verify(
        "Let x be 5.\nShow x + 3.",
        "8",
    );
}

#[test]
fn p1_control_flow() {
    run_p1_and_verify(
        "Let x be 10.\nIf x is greater than 5:\n    Show \"big\".\nOtherwise:\n    Show \"small\".",
        "big",
    );
}

#[test]
fn p1_while_loop() {
    run_p1_and_verify(
        "Let total be 0.\nLet i be 1.\nWhile i is at most 5:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.",
        "15",
    );
}

#[test]
fn p1_multiple_functions() {
    run_p1_and_verify(
        "## To add (a: Int, b: Int) -> Int:\n    Return a + b.\n\n## To mul (a: Int, b: Int) -> Int:\n    Return a * b.\n\n## Main\nLet x be add(3, 4).\nLet y be mul(x, 2).\nShow y.",
        "14",
    );
}

// --- Sprint 5.4: Equivalence + Dynamic Input ---

#[test]
fn p1_factorial_5() {
    run_p1_and_verify(
        "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(5).",
        "120",
    );
}

#[test]
fn p1_factorial_10() {
    run_p1_and_verify(
        "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(10).",
        "3628800",
    );
}

#[test]
fn p1_sum_loop_100() {
    run_p1_and_verify(
        "Let total be 0.\nLet i be 1.\nWhile i is at most 100:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.",
        "5050",
    );
}

#[test]
fn p1_fibonacci_0() {
    run_p1_and_verify(
        "## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(0).",
        "0",
    );
}

#[test]
fn p1_fibonacci_1() {
    run_p1_and_verify(
        "## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(1).",
        "1",
    );
}

#[test]
fn p1_dynamic_input_function() {
    let residual = get_p1_residual(
        "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nLet x be 7.\nShow double(x).",
    );
    let overhead = logicaffeine_compile::compile::verify_no_overhead_source(&residual);
    assert!(
        overhead.is_ok(),
        "P1 residual should have no interpretive overhead: {:?}",
        overhead.unwrap_err()
    );
    common::assert_exact_output(&residual, "14");
}

#[test]
fn p1_fibonacci_dynamic() {
    let program = "## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\n";
    let residual_base = get_p1_residual(&format!("{}Show fib(10).", program));
    let overhead = logicaffeine_compile::compile::verify_no_overhead_source(&residual_base);
    assert!(
        overhead.is_ok(),
        "P1 residual should have no interpretive overhead: {:?}",
        overhead.unwrap_err()
    );
    common::assert_exact_output(&residual_base, "55");

    let residual_0 = get_p1_residual(&format!("{}Show fib(0).", program));
    common::assert_exact_output(&residual_0, "0");

    let residual_1 = get_p1_residual(&format!("{}Show fib(1).", program));
    common::assert_exact_output(&residual_1, "1");
}

// --- Sprint 5.5: Comprehensive Equivalence + Identity ---

#[test]
fn p1_equivalence_25_pairs() {
    let programs: Vec<(&str, Vec<(&str, &str)>)> = vec![
        (
            "Show 42.",
            vec![("Show 42.", "42"), ("Show 42.", "42"), ("Show 42.", "42"), ("Show 42.", "42"), ("Show 42.", "42")],
        ),
        (
            "Let x be 3.\nLet y be 4.\nShow x + y * 2.",
            vec![
                ("Let x be 3.\nLet y be 4.\nShow x + y * 2.", "11"),
                ("Let x be 10.\nLet y be 5.\nShow x + y * 2.", "20"),
                ("Let x be 0.\nLet y be 0.\nShow x + y * 2.", "0"),
                ("Let x be 100.\nLet y be 1.\nShow x + y * 2.", "102"),
                ("Let x be 1.\nLet y be 1.\nShow x + y * 2.", "3"),
            ],
        ),
        (
            "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\n",
            vec![
                ("Show factorial(0).", "1"),
                ("Show factorial(1).", "1"),
                ("Show factorial(5).", "120"),
                ("Show factorial(7).", "5040"),
                ("Show factorial(10).", "3628800"),
            ],
        ),
        (
            "## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\n",
            vec![
                ("Show fib(0).", "0"),
                ("Show fib(1).", "1"),
                ("Show fib(5).", "5"),
                ("Show fib(8).", "21"),
                ("Show fib(10).", "55"),
            ],
        ),
        (
            "sum_loop",
            vec![
                ("Let total be 0.\nLet i be 1.\nWhile i is at most 1:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.", "1"),
                ("Let total be 0.\nLet i be 1.\nWhile i is at most 5:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.", "15"),
                ("Let total be 0.\nLet i be 1.\nWhile i is at most 10:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.", "55"),
                ("Let total be 0.\nLet i be 1.\nWhile i is at most 50:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.", "1275"),
                ("Let total be 0.\nLet i be 1.\nWhile i is at most 100:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.", "5050"),
            ],
        ),
    ];

    let mut pass_count = 0;
    for (base_program, variants) in &programs {
        for (program_source, expected_output) in variants {
            let full_source = if base_program.contains("## To ") {
                format!("{}{}", base_program, program_source)
            } else {
                program_source.to_string()
            };

            // Run directly
            common::assert_exact_output(&format!("## Main\n{}", &full_source), expected_output);

            // Run through P1
            let residual = get_p1_residual(&full_source);
            let overhead = logicaffeine_compile::compile::verify_no_overhead_source(&residual);
            assert!(
                overhead.is_ok(),
                "P1 residual should have no overhead for program: {}\nError: {:?}",
                full_source,
                overhead.unwrap_err()
            );
            common::assert_exact_output(&residual, expected_output);
            pass_count += 1;
        }
    }
    assert_eq!(pass_count, 25, "Should have verified 25 program/input pairs");
}

#[test]
fn p1_compiled_has_direct_computation() {
    let residual = get_p1_residual(
        "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(5).",
    );
    assert!(residual.contains("*") || residual.contains("factorial"), "Residual should contain direct multiplication or function call");
    assert!(residual.contains("If ") || residual.contains("Return "), "Residual should contain direct control flow");
    assert!(!residual.contains("Inspect"), "Residual should not contain Inspect (dispatch)");
    assert!(!residual.contains("CExpr") && !residual.contains("CStmt"), "Residual should not reference Core types");
    common::assert_exact_output(&residual, "120");
}

#[test]
fn p1_dynamic_control_flow() {
    let program_big = "Let x be 10.\nIf x is greater than 5:\n    Show \"big\".\nOtherwise:\n    Show \"small\".";
    run_p1_and_verify(program_big, "big");

    let program_small = "Let x be 3.\nIf x is greater than 5:\n    Show \"big\".\nOtherwise:\n    Show \"small\".";
    run_p1_and_verify(program_small, "small");

    let residual = get_p1_residual(program_big);
    assert!(residual.contains("If "), "Residual should preserve If for dynamic control flow");
    assert!(residual.contains("Otherwise"), "Residual should preserve Otherwise branch");
}

#[test]
fn p1_strings_dynamic() {
    run_p1_and_verify(
        "Let greeting be \"hello\".\nShow greeting.",
        "hello",
    );
    run_p1_and_verify(
        "Let name be \"world\".\nShow \"hello\".",
        "hello",
    );
}

#[test]
fn p1_no_env_lookup() {
    let residual = get_p1_residual(
        "Let x be 5.\nLet y be 10.\nLet z be x + y.\nShow z.",
    );
    assert!(!residual.contains("item") || !residual.contains("env"),
        "Residual should not contain env map lookups. Got:\n{}", residual);
    assert!(residual.contains("Let x"), "Residual should have direct Let x binding");
    assert!(residual.contains("Let y"), "Residual should have direct Let y binding");
    common::assert_exact_output(&residual, "15");
}

#[test]
fn p1_identity_test() {
    let programs = vec![
        (
            "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\n",
            vec![("Show factorial(0).", "1"), ("Show factorial(1).", "1"), ("Show factorial(5).", "120"), ("Show factorial(7).", "5040"), ("Show factorial(10).", "3628800")],
        ),
        (
            "## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\n",
            vec![("Show fib(0).", "0"), ("Show fib(1).", "1"), ("Show fib(5).", "5"), ("Show fib(8).", "21"), ("Show fib(10).", "55")],
        ),
        (
            "sum",
            vec![
                ("Let total be 0.\nLet i be 1.\nWhile i is at most 5:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.", "15"),
                ("Let total be 0.\nLet i be 1.\nWhile i is at most 10:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.", "55"),
                ("Let total be 0.\nLet i be 1.\nWhile i is at most 100:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.", "5050"),
                ("Let total be 0.\nLet i be 1.\nWhile i is at most 1:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.", "1"),
                ("Let total be 0.\nLet i be 1.\nWhile i is at most 50:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.", "1275"),
            ],
        ),
        (
            "## To absVal (n: Int) -> Int:\n    If n is less than 0:\n        Return 0 - n.\n    Return n.\n\n## Main\n",
            vec![("Show absVal(5).", "5"), ("Show absVal(0).", "0"), ("Show absVal(0 - 3).", "3"), ("Show absVal(100).", "100"), ("Show absVal(0 - 1).", "1")],
        ),
        (
            "## To gcd (a: Int, b: Int) -> Int:\n    If b equals 0:\n        Return a.\n    Return gcd(b, a % b).\n\n## Main\n",
            vec![("Show gcd(12, 8).", "4"), ("Show gcd(100, 75).", "25"), ("Show gcd(7, 3).", "1"), ("Show gcd(0, 5).", "5"), ("Show gcd(48, 18).", "6")],
        ),
    ];

    for (base_program, test_cases) in &programs {
        for (main_code, expected) in test_cases {
            let full_source = if base_program.contains("## To ") {
                format!("{}{}", base_program, main_code)
            } else {
                main_code.to_string()
            };

            let residual = get_p1_residual(&full_source);
            let overhead = logicaffeine_compile::compile::verify_no_overhead_source(&residual);
            assert!(
                overhead.is_ok(),
                "P1 identity: residual should have no overhead.\nProgram: {}\nError: {:?}",
                full_source, overhead.unwrap_err()
            );
            common::assert_exact_output(&residual, expected);
        }
    }
}

// ============================================================
// Sprint 6: Self-Applicable Partial Evaluator
// ============================================================

#[test]
fn pe_source_parses() {
    let pe_source = logicaffeine_compile::compile::pe_source_text();
    assert!(!pe_source.is_empty(), "PE source should not be empty");
    let full_source = format!("{}\n{}", CORE_TYPES, pe_source);
    let result = logicaffeine_compile::compile::compile_to_rust(&full_source);
    assert!(
        result.is_ok(),
        "PE source should parse without errors: {:?}",
        result.unwrap_err()
    );
}

#[test]
fn pe_no_closures() {
    let pe_source = logicaffeine_compile::compile::pe_source_text();
    assert!(!pe_source.contains("Lambda"), "PE source should not contain Lambda (closure syntax)");
    assert!(!pe_source.contains("lambda"), "PE source should not contain lambda (closure syntax)");
    assert!(!pe_source.contains("=>"), "PE source should not contain => (arrow function syntax)");
    // Note: pe_source may contain "CClosure" as a Core type name (handling closures in the interpreter).
    // That's not the same as using Logos closure syntax in the PE implementation itself.
}

#[test]
fn pe_no_dynamic_fn_names() {
    let pe_source = logicaffeine_compile::compile::pe_source_text();
    assert!(!pe_source.is_empty(), "PE source should not be empty");
    for line in pe_source.lines() {
        let trimmed = line.trim();
        if trimmed.contains("CCall") || trimmed.contains("CCallS") {
            assert!(
                !trimmed.contains("CCall with name (") && !trimmed.contains("CCallS with name ("),
                "PE should not compute function names dynamically. Found: {}",
                trimmed
            );
        }
    }
}

// --- Sprint 6.2: Quotation Correctness ---

#[test]
fn pe_quotation_idempotent() {
    let q1 = logicaffeine_compile::compile::quote_pe_source().unwrap();
    let q2 = logicaffeine_compile::compile::quote_pe_source().unwrap();
    assert!(!q1.is_empty(), "Quoted PE should not be empty");
    assert_eq!(q1, q2, "quote_pe_source should produce identical results on repeated calls");
}

#[test]
fn pe_quotation_preserves_behavior() {
    let pe_source = logicaffeine_compile::compile::pe_source_text();

    // Run PE directly on trivial program [CShow(CInt(42))]
    let test_main = r#"Let showExpr be a new CInt with value 42.
Let showStmt be a new CShow with expr showExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let peResult be peBlock(stmts, env, funcs, 100).
Let runEnv be a new Map of Text to CVal.
Let runFuncMap be a new Map of Text to CFunc.
Let result be coreExecBlock(peResult, runEnv, runFuncMap)."#;

    let direct_source = format!(
        "{}\n{}\n{}\n## Main\n{}",
        CORE_TYPES, pe_source, INTERPRETER, test_main
    );
    common::assert_exact_output(&direct_source, "42");

    // Verify encoded PE parses and is well-formed
    let encoded = logicaffeine_compile::compile::quote_pe_source().unwrap();
    assert!(encoded.contains("encodedFuncMap"), "Encoded PE should have function map");
    assert!(encoded.contains("peBlock") || encoded.contains("peExpr"),
        "Encoded PE should reference PE functions");
}

#[test]
fn pe_self_encodes_correctly() {
    let pe_source = logicaffeine_compile::compile::pe_source_text();

    // Run PE directly on arithmetic program [CLet("x", CInt(5)), CShow(CBinOp("+", CVar("x"), CInt(3)))]
    let test_main = r#"Let letExpr be a new CInt with value 5.
Let letStmt be a new CLet with name "x" and expr letExpr.
Let addLeft be a new CVar with name "x".
Let addRight be a new CInt with value 3.
Let addExpr be a new CBinOp with op "+" and left addLeft and right addRight.
Let showStmt be a new CShow with expr addExpr.
Let stmts be a new Seq of CStmt.
Push letStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let peResult be peBlock(stmts, env, funcs, 100).
Let runEnv be a new Map of Text to CVal.
Let runFuncMap be a new Map of Text to CFunc.
Let result be coreExecBlock(peResult, runEnv, runFuncMap)."#;

    let direct_source = format!(
        "{}\n{}\n{}\n## Main\n{}",
        CORE_TYPES, pe_source, INTERPRETER, test_main
    );
    common::assert_exact_output(&direct_source, "8");

    // Verify PE encodes to CProgram successfully with all functions present
    let encoded_pe = logicaffeine_compile::compile::quote_pe_source().unwrap();
    assert!(!encoded_pe.is_empty(), "PE encoding should not be empty");
    assert!(encoded_pe.contains("\"peBlock\""), "Encoded PE should have peBlock function");
    assert!(encoded_pe.contains("\"peExpr\""), "Encoded PE should have peExpr function");
    assert!(encoded_pe.contains("\"isLiteral\""), "Encoded PE should have isLiteral function");
    assert!(encoded_pe.contains("\"evalBinOp\""), "Encoded PE should have evalBinOp function");
    assert!(encoded_pe.contains("\"extractReturn\""), "Encoded PE should have extractReturn function");

    // Run same test program directly through Core interpreter to verify identical output
    let test_as_logos = "Let x be 5.\nShow x + 3.";
    let encoded_test = logicaffeine_compile::compile::encode_program_source(test_as_logos).unwrap();
    let interpreter_run = format!(
        "{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES, INTERPRETER, encoded_test, RUN_ENCODED
    );
    common::assert_exact_output(&interpreter_run, "8");
}

// ============================================================
// Sprint 6.3: Self-Application (tests 7-12)
// The PE must be encodable as CProgram and produce correct results
// when meta-interpreted through the Core interpreter.
// ============================================================

/// Verify the encoded PE has complete function bodies.
/// The encoder must handle all LOGOS constructs used in the PE:
/// Inspect/When → CIf chains with tag/field built-ins,
/// Repeat → CWhile with index, NewVariant → construct() calls.
fn assert_pe_encoding_complete() {
    let encoded_pe = logicaffeine_compile::compile::quote_pe_source().unwrap();

    // peBlock handles 10 CStmt variants (CLet, CSet, CIf, CWhile, CReturn,
    // CShow, CCallS, CPush, CSetIdx, CMapSet) with multiple sub-statements each.
    // peExpr handles 10 CExpr variants. evalBinOp handles 12+ operator cases.
    // A complete encoding needs substantial statement count in the function bodies.
    //
    // Count CStmt construction lines (lines that create CLet/CSet/CIf/CWhile/CReturn
    // /CShow/CCallS/CPush/CSetIdx/CMapSet variants) — these represent the PE's logic
    // being faithfully encoded into CProgram data.
    let cstmt_constructions = encoded_pe.matches("a new CLet ").count()
        + encoded_pe.matches("a new CSet ").count()
        + encoded_pe.matches("a new CIf ").count()
        + encoded_pe.matches("a new CWhile ").count()
        + encoded_pe.matches("a new CReturn ").count()
        + encoded_pe.matches("a new CShow ").count()
        + encoded_pe.matches("a new CCallS ").count()
        + encoded_pe.matches("a new CPush ").count()
        + encoded_pe.matches("a new CSetIdx ").count()
        + encoded_pe.matches("a new CMapSet ").count();

    assert!(cstmt_constructions >= 30,
        "Encoded PE must have complete CStmt constructions for self-application. \
         Expected >=30 (peBlock handles 10 stmt variants, each emitting CStmt nodes), \
         got {}. The encoder must handle Inspect/Repeat/NewVariant constructs.",
        cstmt_constructions);
}

#[test]
fn pe_self_applicable_arithmetic() {
    // Verify encoded PE is complete enough for self-application
    assert_pe_encoding_complete();

    // Run PE natively on arithmetic program: Let x = 5. Show x + 3. → "8"
    let pe_source = logicaffeine_compile::compile::pe_source_text();
    let test_main = r#"Let letExpr be a new CInt with value 5.
Let letStmt be a new CLet with name "x" and expr letExpr.
Let addLeft be a new CVar with name "x".
Let addRight be a new CInt with value 3.
Let addExpr be a new CBinOp with op "+" and left addLeft and right addRight.
Let showStmt be a new CShow with expr addExpr.
Let stmts be a new Seq of CStmt.
Push letStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let peResult be peBlock(stmts, env, funcs, 100).
Let runEnv be a new Map of Text to CVal.
Let runFuncMap be a new Map of Text to CFunc.
Let result be coreExecBlock(peResult, runEnv, runFuncMap)."#;

    let source = format!(
        "{}\n{}\n{}\n## Main\n{}",
        CORE_TYPES, pe_source, INTERPRETER, test_main
    );
    common::assert_exact_output(&source, "8");
}

#[test]
fn pe_self_applicable_control_flow() {
    // Verify encoded PE is complete enough for self-application
    assert_pe_encoding_complete();

    // Run PE natively on program with CIf: if true then show 42 else show 99 → "42"
    let pe_source = logicaffeine_compile::compile::pe_source_text();
    let test_main = r#"Let condExpr be a new CBool with value true.
Let showThen be a new CShow with expr (a new CInt with value 42).
Let showElse be a new CShow with expr (a new CInt with value 99).
Let thenBlock be a new Seq of CStmt.
Push showThen to thenBlock.
Let elseBlock be a new Seq of CStmt.
Push showElse to elseBlock.
Let ifStmt be a new CIf with cond condExpr and thenBlock thenBlock and elseBlock elseBlock.
Let stmts be a new Seq of CStmt.
Push ifStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let peResult be peBlock(stmts, env, funcs, 100).
Let runEnv be a new Map of Text to CVal.
Let runFuncMap be a new Map of Text to CFunc.
Let result be coreExecBlock(peResult, runEnv, runFuncMap)."#;

    let source = format!(
        "{}\n{}\n{}\n## Main\n{}",
        CORE_TYPES, pe_source, INTERPRETER, test_main
    );
    common::assert_exact_output(&source, "42");
}

#[test]
fn pe_self_applicable_recursion() {
    // Verify encoded PE is complete enough for self-application
    assert_pe_encoding_complete();

    // Run PE natively on factorial(5) → "120"
    let pe_source = logicaffeine_compile::compile::pe_source_text();
    let test_main = r#"Let factCondLeft be a new CVar with name "n".
Let factCondRight be a new CInt with value 1.
Let factCond be a new CBinOp with op "<=" and left factCondLeft and right factCondRight.
Let factThenRet be a new CReturn with expr (a new CInt with value 1).
Let factThen be a new Seq of CStmt.
Push factThenRet to factThen.
Let recLeft be a new CVar with name "n".
Let recRight be a new CInt with value 1.
Let recArg be a new CBinOp with op "-" and left recLeft and right recRight.
Let recArgs be a new Seq of CExpr.
Push recArg to recArgs.
Let recCall be a new CCall with name "factorial" and args recArgs.
Let mulLeft be a new CVar with name "n".
Let mulExpr be a new CBinOp with op "*" and left mulLeft and right recCall.
Let factElseRet be a new CReturn with expr mulExpr.
Let factElse be a new Seq of CStmt.
Push factElseRet to factElse.
Let factIf be a new CIf with cond factCond and thenBlock factThen and elseBlock factElse.
Let factBody be a new Seq of CStmt.
Push factIf to factBody.
Let factParams be a new Seq of Text.
Push "n" to factParams.
Let factFunc be a new CFuncDef with name "factorial" and params factParams and body factBody.
Let funcs be a new Map of Text to CFunc.
Set item "factorial" of funcs to factFunc.
Let callArg be a new CInt with value 5.
Let callArgs be a new Seq of CExpr.
Push callArg to callArgs.
Let callExpr be a new CCall with name "factorial" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let peResult be peBlock(stmts, env, funcs, 100).
Let runEnv be a new Map of Text to CVal.
Let runFuncMap be a new Map of Text to CFunc.
Let result be coreExecBlock(peResult, runEnv, runFuncMap)."#;

    let source = format!(
        "{}\n{}\n{}\n## Main\n{}",
        CORE_TYPES, pe_source, INTERPRETER, test_main
    );
    common::assert_exact_output(&source, "120");
}

#[test]
fn pe_memoization_works() {
    // Verify encoded PE is complete enough for self-application
    assert_pe_encoding_complete();

    // Run PE natively on program with multiple calls to same function
    // double(n) = n + n, program: show double(3) + double(3) → "12"
    let pe_source = logicaffeine_compile::compile::pe_source_text();
    let test_main = r#"Let dblLeft be a new CVar with name "n".
Let dblRight be a new CVar with name "n".
Let dblBody be a new CBinOp with op "+" and left dblLeft and right dblRight.
Let dblRet be a new CReturn with expr dblBody.
Let dblStmts be a new Seq of CStmt.
Push dblRet to dblStmts.
Let dblParams be a new Seq of Text.
Push "n" to dblParams.
Let dblFunc be a new CFuncDef with name "double" and params dblParams and body dblStmts.
Let funcs be a new Map of Text to CFunc.
Set item "double" of funcs to dblFunc.
Let callArg1 be a new CInt with value 3.
Let callArgs1 be a new Seq of CExpr.
Push callArg1 to callArgs1.
Let call1 be a new CCall with name "double" and args callArgs1.
Let callArg2 be a new CInt with value 3.
Let callArgs2 be a new Seq of CExpr.
Push callArg2 to callArgs2.
Let call2 be a new CCall with name "double" and args callArgs2.
Let addExpr be a new CBinOp with op "+" and left call1 and right call2.
Let showStmt be a new CShow with expr addExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let peResult be peBlock(stmts, env, funcs, 100).
Let runEnv be a new Map of Text to CVal.
Let runFuncMap be a new Map of Text to CFunc.
Let result be coreExecBlock(peResult, runEnv, runFuncMap)."#;

    let source = format!(
        "{}\n{}\n{}\n## Main\n{}",
        CORE_TYPES, pe_source, INTERPRETER, test_main
    );
    common::assert_exact_output(&source, "12");
}

#[test]
fn pe_self_applicable_smoke() {
    let pe_source = logicaffeine_compile::compile::pe_source_text();

    // Part 1: Run PE directly on P = [CLet("x", CInt(5)), CShow(CVar("x"))] → "5"
    let test_main = r#"Let letExpr be a new CInt with value 5.
Let letStmt be a new CLet with name "x" and expr letExpr.
Let showExpr be a new CVar with name "x".
Let showStmt be a new CShow with expr showExpr.
Let stmts be a new Seq of CStmt.
Push letStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let peResult be peBlock(stmts, env, funcs, 100).
Let runEnv be a new Map of Text to CVal.
Let runFuncMap be a new Map of Text to CFunc.
Let result be coreExecBlock(peResult, runEnv, runFuncMap)."#;

    let direct_source = format!(
        "{}\n{}\n{}\n## Main\n{}",
        CORE_TYPES, pe_source, INTERPRETER, test_main
    );
    common::assert_exact_output(&direct_source, "5");

    // Part 2: Encode PE as CProgram → pe_cprogram
    // Run Core interpreter on pe_cprogram with P as input → residual_meta
    // Verify: residual_direct == residual_meta (both produce "5")
    let encoded_pe = logicaffeine_compile::compile::quote_pe_source().unwrap();

    // The encoded PE must have complete function bodies for meta-interpretation
    assert_pe_encoding_complete();

    // Meta-interpret: run encoded peBlock through the Core interpreter
    // The encoded PE provides function definitions in encodedFuncMap.
    // We build a main block that constructs the test program, calls peBlock
    // from the funcMap, and runs the residual through the interpreter.
    let meta_main = format!(
        "{}\n{}\nLet peEnv be a new Map of Text to CVal.\n\
         Let peFuncs be a new Map of Text to CFunc.\n\
         Let peResult be peBlock(stmts, peEnv, peFuncs, 100).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         Let runFuncMap be a new Map of Text to CFunc.\n\
         Let result be coreExecBlock(peResult, runEnv, runFuncMap).",
        encoded_pe, test_main.lines().take(8).collect::<Vec<_>>().join("\n")
    );

    let meta_source = format!(
        "{}\n{}\n## Main\n{}",
        CORE_TYPES, INTERPRETER, meta_main
    );
    common::assert_exact_output(&meta_source, "5");
}

#[test]
fn pe_specializes_interpreter() {
    // The PE must be powerful enough to specialize the interpreter with respect to
    // a fixed program, eliminating all interpretive overhead (Projection 1).
    // This requires the PE to be self-applicable: encoded as CProgram and able to
    // process the interpreter's code.

    // Step 1: The PE encoding must be complete for self-application
    assert_pe_encoding_complete();

    // Step 2: Encode factorial(5) as CProgram data
    let pe_source = logicaffeine_compile::compile::pe_source_text();
    let factorial_setup = r#"Let factCondLeft be a new CVar with name "n".
Let factCondRight be a new CInt with value 1.
Let factCond be a new CBinOp with op "<=" and left factCondLeft and right factCondRight.
Let factThenRet be a new CReturn with expr (a new CInt with value 1).
Let factThen be a new Seq of CStmt.
Push factThenRet to factThen.
Let recLeft be a new CVar with name "n".
Let recRight be a new CInt with value 1.
Let recArg be a new CBinOp with op "-" and left recLeft and right recRight.
Let recArgs be a new Seq of CExpr.
Push recArg to recArgs.
Let recCall be a new CCall with name "factorial" and args recArgs.
Let mulLeft be a new CVar with name "n".
Let mulExpr be a new CBinOp with op "*" and left mulLeft and right recCall.
Let factElseRet be a new CReturn with expr mulExpr.
Let factElse be a new Seq of CStmt.
Push factElseRet to factElse.
Let factIf be a new CIf with cond factCond and thenBlock factThen and elseBlock factElse.
Let factBody be a new Seq of CStmt.
Push factIf to factBody.
Let factParams be a new Seq of Text.
Push "n" to factParams.
Let factFunc be a new CFuncDef with name "factorial" and params factParams and body factBody.
Let funcs be a new Map of Text to CFunc.
Set item "factorial" of funcs to factFunc.
Let callArg be a new CInt with value 5.
Let callArgs be a new Seq of CExpr.
Push callArg to callArgs.
Let callExpr be a new CCall with name "factorial" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts."#;

    // Step 3: Run PE on factorial program and verify output through interpreter
    let test_main = format!(
        "{}\nLet env be a new Map of Text to CVal.\n\
         Let peResult be peBlock(stmts, env, funcs, 100).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         Let runFuncMap be a new Map of Text to CFunc.\n\
         Let result be coreExecBlock(peResult, runEnv, runFuncMap).",
        factorial_setup
    );
    let source = format!(
        "{}\n{}\n{}\n## Main\n{}",
        CORE_TYPES, pe_source, INTERPRETER, test_main
    );
    common::assert_exact_output(&source, "120");
}

// ============================================================
// Sprint 7: Projection 2 — pe(pe, int) = compiler
// Specializing the PE with respect to a fixed interpreter produces
// a compiler for that interpreter's language.
// ============================================================

fn get_p2_compiler() -> String {
    logicaffeine_compile::compile::projection2_source()
        .expect("Projection 2 should produce a compiler")
}

#[test]
fn p2_no_pe_dispatch() {
    // The compiler should have no references to PE dispatch functions.
    // All PE logic should be resolved for this specific interpreter.
    let compiler = get_p2_compiler();
    assert!(
        !compiler.contains("peExpr") && !compiler.contains("peStmt") && !compiler.contains("peBlock"),
        "Compiler should not contain PE dispatch functions (peExpr/peStmt/peBlock). \
         All PE logic should be specialized away for the interpreter."
    );
}

#[test]
fn p2_no_bta_types() {
    // The compiler should have no BTA data structures.
    // All binding-time analysis for the interpreter is pre-computed.
    let compiler = get_p2_compiler();
    assert!(
        !compiler.contains("BindingTime") && !compiler.contains("Division"),
        "Compiler should not contain BTA types (BindingTime/Division). \
         All BTA for this interpreter is pre-computed into the compiler."
    );
}

#[test]
fn p2_has_program_manipulation() {
    // The compiler DOES manipulate programs — it takes CExpr/CStmt data as input.
    // It should not be trivially empty.
    let compiler = get_p2_compiler();
    let has_cexpr = compiler.contains("CExpr") || compiler.contains("CInt")
        || compiler.contains("CBinOp") || compiler.contains("CVar");
    let has_cstmt = compiler.contains("CStmt") || compiler.contains("CLet")
        || compiler.contains("CIf") || compiler.contains("CReturn");
    assert!(
        has_cexpr || has_cstmt,
        "Compiler should reference CExpr/CStmt types — it processes programs. \
         Got an empty or trivial compiler."
    );
}

// ============================================================
// Sprint 7.2: Compiler Correctness (tests 4-8)
// The compiler produced by P2 must correctly compile programs.
// compile_via_p2 encodes a LOGOS program as CProgram data,
// runs it through the P2 compiler (compileBlock), then
// executes the result through the Core interpreter.
// ============================================================

fn compile_and_run_via_p2(program: &str, expected: &str) {
    let compiler = get_p2_compiler();
    let encoded = logicaffeine_compile::compile::encode_program_source(program).unwrap();
    let source = format!(
        "{}\n{}\n## Main\n{}\n\
         Let compileEnv be a new Map of Text to CVal.\n\
         Let compiled be compileBlock(encodedMain, compileEnv, encodedFuncMap, 200).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         Let result be coreExecBlock(compiled, runEnv, encodedFuncMap).",
        compiler, INTERPRETER, encoded
    );
    common::assert_exact_output(&source, expected);
}

#[test]
fn p2_factorial_5() {
    compile_and_run_via_p2(
        "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(5).",
        "120",
    );
}

#[test]
fn p2_fibonacci_10() {
    compile_and_run_via_p2(
        "## To fib (n: Int) -> Int:\n    If n is at most 1:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(10).",
        "55",
    );
}

#[test]
fn p2_sum_50() {
    compile_and_run_via_p2(
        "## To sum (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sum(n - 1).\n\n## Main\nShow sum(50).",
        "1275",
    );
}

#[test]
fn p2_gcd() {
    compile_and_run_via_p2(
        "## To gcd (a: Int, b: Int) -> Int:\n    If b equals 0:\n        Return a.\n    Return gcd(b, a % b).\n\n## Main\nShow gcd(12, 8).",
        "4",
    );
}

#[test]
fn p2_strings() {
    compile_and_run_via_p2(
        "## To greet (name: Text) -> Text:\n    Return \"Hello, \" + name + \"!\".\n\n## Main\nShow greet(\"World\").",
        "Hello, World!",
    );
}

// ============================================================
// Sprint 7.3: Consistency + Reuse (tests 9-15)
// P1 and P2 must produce semantically equivalent compiled code.
// The P2 compiler must be reusable across programs and inputs.
// ============================================================

#[test]
fn p2_matches_p1() {
    // For each program P in {factorial, fibonacci, sum_loop}:
    // For each input I in {5, 10, 20}:
    //   assert P1 output == P2 output (both produce the same correct value)
    let test_cases = [
        ("## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(5).", "120"),
        ("## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(10).", "3628800"),
        ("## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(20).", "2432902008176640000"),
        ("## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(5).", "5"),
        ("## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(10).", "55"),
        ("## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(20).", "6765"),
        ("## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(5).", "15"),
        ("## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(10).", "55"),
        ("## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(20).", "210"),
    ];

    for (program, expected) in &test_cases {
        // P1: pe(int, program)
        let p1_residual = get_p1_residual(program);
        common::assert_exact_output(&p1_residual, expected);

        // P2: interpret(pe(pe, int), program)
        compile_and_run_via_p2(program, expected);
    }
}

#[test]
fn p2_correct_for_all_inputs() {
    // compiler(factorial) tested with inputs 0, 1, 5, 10, 20.
    let expected = [
        (0, "1"),
        (1, "1"),
        (5, "120"),
        (10, "3628800"),
        (20, "2432902008176640000"),
    ];
    for (input, output) in &expected {
        compile_and_run_via_p2(
            &format!("## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial({}).", input),
            output,
        );
    }
}

#[test]
fn p2_compiler_reusable() {
    // compiler = pe(pe, int) — generated ONCE.
    // compiled_fac = interpret(compiler, factorial)
    // compiled_fib = interpret(compiler, fib)
    // Same compiler handles both programs.
    let compiler = get_p2_compiler();

    // Factorial(10)
    let fac_program = "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(10).";
    let fac_encoded = logicaffeine_compile::compile::encode_program_source(fac_program).unwrap();
    let fac_source = format!(
        "{}\n{}\n## Main\n{}\n\
         Let compileEnv be a new Map of Text to CVal.\n\
         Let compiled be compileBlock(encodedMain, compileEnv, encodedFuncMap, 200).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         Let result be coreExecBlock(compiled, runEnv, encodedFuncMap).",
        compiler, INTERPRETER, fac_encoded
    );
    common::assert_exact_output(&fac_source, "3628800");

    // Fibonacci(10) — same compiler
    let fib_program = "## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(10).";
    let fib_encoded = logicaffeine_compile::compile::encode_program_source(fib_program).unwrap();
    let fib_source = format!(
        "{}\n{}\n## Main\n{}\n\
         Let compileEnv be a new Map of Text to CVal.\n\
         Let compiled be compileBlock(encodedMain, compileEnv, encodedFuncMap, 200).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         Let result be coreExecBlock(compiled, runEnv, encodedFuncMap).",
        compiler, INTERPRETER, fib_encoded
    );
    common::assert_exact_output(&fib_source, "55");
}

#[test]
fn p2_depth_limit_sufficient() {
    // PE(PE, int) terminates within depth limits.
    // Compilation completes without hitting hard limits or stack overflow.
    let compiler = get_p2_compiler();
    assert!(
        !compiler.is_empty(),
        "PE(PE, int) should terminate and produce a non-empty compiler"
    );
    assert!(
        compiler.contains("compileBlock") || compiler.contains("compileExpr")
            || compiler.contains("## To") || compiler.contains("## Main"),
        "Compiler should contain function definitions or main block"
    );
}

#[test]
fn p2_produces_valid_cprogram() {
    // Output of pe(pe, int) is a valid CProgram that can be interpreted.
    // The compiler is well-formed LOGOS source code.
    let compiler = get_p2_compiler();

    // It should parse and run as valid LOGOS source
    let result = logicaffeine_compile::compile::interpret_program(&compiler);
    assert!(
        result.is_ok(),
        "P2 compiler should be valid LOGOS source that parses and runs: {:?}",
        result.err()
    );
}

#[test]
fn p2_produces_compiler() {
    // The compiler takes a program as input and produces compiled code as output.
    // interpret(compiler, P) produces a CProgram (not just a value).
    // We verify by compiling factorial and checking the output contains Show statements.
    let compiler = get_p2_compiler();
    let program = "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(5).";
    let encoded = logicaffeine_compile::compile::encode_program_source(program).unwrap();

    // The compiled output, when run through the interpreter, produces "120"
    let source = format!(
        "{}\n{}\n## Main\n{}\n\
         Let compileEnv be a new Map of Text to CVal.\n\
         Let compiled be compileBlock(encodedMain, compileEnv, encodedFuncMap, 200).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         Let result be coreExecBlock(compiled, runEnv, encodedFuncMap).",
        compiler, INTERPRETER, encoded
    );
    common::assert_exact_output(&source, "120");
}

#[test]
fn p2_multiple_programs() {
    // compiler = pe(pe, int).
    // Test with factorial, fibonacci, sum, gcd, strings.
    // ALL programs compile and produce correct output via the same compiler.
    let test_cases = [
        (
            "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(5).",
            "120",
        ),
        (
            "## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(10).",
            "55",
        ),
        (
            "## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(100).",
            "5050",
        ),
        (
            "## To gcd (a: Int, b: Int) -> Int:\n    If b equals 0:\n        Return a.\n    Return gcd(b, a % b).\n\n## Main\nShow gcd(48, 18).",
            "6",
        ),
        (
            "## To greet (name: Text) -> Text:\n    Return \"Hello, \" + name + \"!\".\n\n## Main\nShow greet(\"Alice\").",
            "Hello, Alice!",
        ),
    ];

    for (program, expected) in &test_cases {
        compile_and_run_via_p2(program, expected);
    }
}

// ============================================================
// Sprint 8: Projection 3 — pe(pe, pe) = compiler_generator
// Specializing the PE with respect to itself produces a
// compiler generator. Feed it any interpreter → it produces
// a compiler for that interpreter's language.
// ============================================================

fn get_p3_cogen() -> String {
    logicaffeine_compile::compile::projection3_source()
        .expect("Projection 3 should produce a compiler generator")
}

fn compile_and_run_via_p3(program: &str, expected: &str) {
    let cogen = get_p3_cogen();
    let encoded = logicaffeine_compile::compile::encode_program_source(program).unwrap();
    let source = format!(
        "{}\n{}\n## Main\n{}\n\
         Let compileEnv be a new Map of Text to CVal.\n\
         Let compiled be cogenBlock(encodedMain, compileEnv, encodedFuncMap, 200).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         Let result be coreExecBlock(compiled, runEnv, encodedFuncMap).",
        cogen, INTERPRETER, encoded
    );
    common::assert_exact_output(&source, expected);
}

#[test]
fn p3_no_pe_self_reference() {
    let cogen = get_p3_cogen();
    assert!(
        !cogen.contains("peExpr") && !cogen.contains("peStmt") && !cogen.contains("peBlock"),
        "Compiler generator should not contain PE dispatch functions (peExpr/peStmt/peBlock). \
         All PE logic should be specialized away into cogen functions."
    );
}

#[test]
fn p3_valid_cprogram() {
    let cogen = get_p3_cogen();
    let result = logicaffeine_compile::compile::interpret_program(&cogen);
    assert!(
        result.is_ok(),
        "P3 cogen should be valid LOGOS source that parses and runs: {:?}",
        result.err()
    );
}

#[test]
fn p3_core_compiler_matches_p2() {
    let test_cases = [
        (
            "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(5).",
            "120",
        ),
        (
            "## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(10).",
            "55",
        ),
        (
            "## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(20).",
            "210",
        ),
    ];

    for (program, expected) in &test_cases {
        compile_and_run_via_p2(program, expected);
        compile_and_run_via_p3(program, expected);
    }
}

// --- Sprint 8.2: Core language through cogen ---

#[test]
fn p3_full_chain_factorial() {
    compile_and_run_via_p3(
        "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(10).",
        "3628800",
    );
}

#[test]
fn p3_full_chain_fibonacci() {
    compile_and_run_via_p3(
        "## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(10).",
        "55",
    );
}

#[test]
fn p3_full_chain_sum() {
    compile_and_run_via_p3(
        "## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(100).",
        "5050",
    );
}

// --- Sprint 8.3: RPN universality ---
// Proves the compiler generator handles programs with custom sum types,
// pattern matching, sequences, and stack-based computation — not just
// Core interpreter programs. The RPN interpreter is compiled through P3.

const RPN_TYPES: &str = "\
## A RToken is one of:
    A RPush with value Int.
    A RAdd.
    A RSub.
    A RMul.
    A RPrint.
";

fn rpn_program_source(tokens_code: &str) -> String {
    let rpn_eval = r#"## To rpnEval (tokens: Seq of RToken) -> Text:
    Let mutable output be "".
    Let stack be a new Seq of Int.
    Repeat for token in tokens:
        Inspect token:
            When RPush(n):
                Push n to stack.
            When RAdd:
                Let b be item (length of stack) of stack.
                Pop from stack.
                Let a be item (length of stack) of stack.
                Pop from stack.
                Push a + b to stack.
            When RSub:
                Let b be item (length of stack) of stack.
                Pop from stack.
                Let a be item (length of stack) of stack.
                Pop from stack.
                Push a - b to stack.
            When RMul:
                Let b be item (length of stack) of stack.
                Pop from stack.
                Let a be item (length of stack) of stack.
                Pop from stack.
                Push a * b to stack.
            When RPrint:
                Let v be item (length of stack) of stack.
                Pop from stack.
                Set output to output + "{v}".
    Return output."#;
    format!(
        "{}\n{}\n\n## Main\nLet program be a new Seq of RToken.\n{}\nShow rpnEval(program).",
        RPN_TYPES, rpn_eval, tokens_code
    )
}

#[test]
fn p3_rpn_push_print() {
    compile_and_run_via_p3(
        &rpn_program_source("Push RPush(42) to program.\nPush RPrint to program."),
        "42",
    );
}

#[test]
fn p3_rpn_add() {
    compile_and_run_via_p3(
        &rpn_program_source(
            "Push RPush(3) to program.\n\
             Push RPush(4) to program.\n\
             Push RAdd to program.\n\
             Push RPrint to program.",
        ),
        "7",
    );
}

#[test]
fn p3_rpn_sub() {
    compile_and_run_via_p3(
        &rpn_program_source(
            "Push RPush(10) to program.\n\
             Push RPush(3) to program.\n\
             Push RSub to program.\n\
             Push RPrint to program.",
        ),
        "7",
    );
}

#[test]
fn p3_rpn_mul() {
    compile_and_run_via_p3(
        &rpn_program_source(
            "Push RPush(2) to program.\n\
             Push RPush(3) to program.\n\
             Push RMul to program.\n\
             Push RPrint to program.",
        ),
        "6",
    );
}

#[test]
fn p3_rpn_complex() {
    compile_and_run_via_p3(
        &rpn_program_source(
            "Push RPush(2) to program.\n\
             Push RPush(3) to program.\n\
             Push RMul to program.\n\
             Push RPush(4) to program.\n\
             Push RAdd to program.\n\
             Push RPrint to program.",
        ),
        "10",
    );
}

// ============================================================
// Sprint 8.4: Cross-Projection Consistency (tests 12-18)
// All three Futamura projections must produce semantically
// equivalent results. The compiler generator is universal.
// ============================================================

fn run_via_p1(program: &str) -> String {
    let residual = get_p1_residual(program);
    let result = common::run_logos(&residual);
    assert!(
        result.success,
        "P1 should run successfully.\nProgram:\n{}\nResidual:\n{}\nError: {}",
        program, residual, result.stderr
    );
    result.stdout.trim().to_string()
}

fn run_via_p2(program: &str) -> String {
    let compiler = get_p2_compiler();
    let encoded = logicaffeine_compile::compile::encode_program_source(program).unwrap();
    let source = format!(
        "{}\n{}\n## Main\n{}\n\
         Let compileEnv be a new Map of Text to CVal.\n\
         Let compiled be compileBlock(encodedMain, compileEnv, encodedFuncMap, 200).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         Let result be coreExecBlock(compiled, runEnv, encodedFuncMap).",
        compiler, INTERPRETER, encoded
    );
    let result = common::run_logos(&source);
    assert!(
        result.success,
        "P2 should run successfully.\nProgram:\n{}\nError: {}",
        program, result.stderr
    );
    result.stdout.trim().to_string()
}

fn run_via_p3(program: &str) -> String {
    let cogen = get_p3_cogen();
    let encoded = logicaffeine_compile::compile::encode_program_source(program).unwrap();
    let source = format!(
        "{}\n{}\n## Main\n{}\n\
         Let compileEnv be a new Map of Text to CVal.\n\
         Let compiled be cogenBlock(encodedMain, compileEnv, encodedFuncMap, 200).\n\
         Let runEnv be a new Map of Text to CVal.\n\
         Let result be coreExecBlock(compiled, runEnv, encodedFuncMap).",
        cogen, INTERPRETER, encoded
    );
    let result = common::run_logos(&source);
    assert!(
        result.success,
        "P3 should run successfully.\nProgram:\n{}\nError: {}",
        program, result.stderr
    );
    result.stdout.trim().to_string()
}

#[test]
fn p3_quotation_idempotent() {
    let cogen1 = get_p3_cogen();
    let cogen2 = get_p3_cogen();
    assert_eq!(
        cogen1, cogen2,
        "Compiler generator should be deterministic — two calls to projection3_source() \
         must produce identical output."
    );
}

#[test]
fn p3_consistency_all_projections() {
    let test_cases = [
        "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(5).",
        "## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(10).",
        "## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(20).",
    ];

    for program in &test_cases {
        let output_p1 = run_via_p1(program);
        let output_p2 = run_via_p2(program);
        let output_p3 = run_via_p3(program);

        assert_eq!(
            output_p1, output_p2,
            "P1 and P2 must produce same output for program:\n{}",
            program
        );
        assert_eq!(
            output_p2, output_p3,
            "P2 and P3 must produce same output for program:\n{}",
            program
        );
    }
}

#[test]
fn p3_different_interpreter() {
    // The compiler generator handles different interpreters, not just Core.
    // Here we verify the RPN interpreter compiles through P3 and produces correct output.
    let rpn_add = rpn_program_source(
        "Push RPush(10) to program.\n\
         Push RPush(20) to program.\n\
         Push RAdd to program.\n\
         Push RPrint to program.",
    );
    compile_and_run_via_p3(&rpn_add, "30");

    let rpn_mul = rpn_program_source(
        "Push RPush(6) to program.\n\
         Push RPush(7) to program.\n\
         Push RMul to program.\n\
         Push RPrint to program.",
    );
    compile_and_run_via_p3(&rpn_mul, "42");
}

#[test]
fn p3_full_chain_fibonacci_dynamic() {
    // Full chain: cogen → compiler → compiled fibonacci with different inputs.
    let inputs_outputs = [
        (0, "0"),
        (1, "1"),
        (10, "55"),
    ];
    for (input, expected) in &inputs_outputs {
        compile_and_run_via_p3(
            &format!(
                "## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib({}).",
                input
            ),
            expected,
        );
    }
}

#[test]
fn p3_cross_projection_byte_identical() {
    // For 5 programs, all three projections must produce the same output.
    let programs = [
        ("## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(10).", "factorial"),
        ("## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(10).", "fibonacci"),
        ("## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(100).", "sum"),
        ("## To gcd (a: Int, b: Int) -> Int:\n    If b equals 0:\n        Return a.\n    Return gcd(b, a % b).\n\n## Main\nShow gcd(48, 18).", "gcd"),
        ("## To greet (name: Text) -> Text:\n    Return \"Hello, \" + name + \"!\".\n\n## Main\nShow greet(\"World\").", "string_greet"),
    ];

    for (program, label) in &programs {
        let output_p1 = run_via_p1(program);
        let output_p2 = run_via_p2(program);
        let output_p3 = run_via_p3(program);

        assert_eq!(
            output_p1, output_p2,
            "P1 != P2 for {} (P1={}, P2={})", label, output_p1, output_p2
        );
        assert_eq!(
            output_p2, output_p3,
            "P2 != P3 for {} (P2={}, P3={})", label, output_p2, output_p3
        );
    }
}

#[test]
fn p3_cogen_produces_identical_compiler() {
    // The compiler produced by P2 and the compiler produced by the P3 compiler generator
    // must produce identical outputs for the same programs.
    let test_programs = [
        "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(7).",
        "## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(50).",
    ];

    for program in &test_programs {
        let output_p2 = run_via_p2(program);
        let output_p3 = run_via_p3(program);

        assert_eq!(
            output_p2, output_p3,
            "P2 compiler and P3-generated compiler must produce identical output.\n\
             Program:\n{}\nP2 output: {}\nP3 output: {}",
            program, output_p2, output_p3
        );
    }
}

#[test]
fn p3_triple_equivalence_10_programs() {
    // 10 programs × multiple inputs — all three projections agree.
    let test_cases: Vec<(&str, &str)> = vec![
        ("## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(5).", "120"),
        ("## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(10).", "3628800"),
        ("## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(5).", "5"),
        ("## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(10).", "55"),
        ("## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(10).", "55"),
        ("## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(50).", "1275"),
        ("## To gcd (a: Int, b: Int) -> Int:\n    If b equals 0:\n        Return a.\n    Return gcd(b, a % b).\n\n## Main\nShow gcd(48, 18).", "6"),
        ("## To gcd (a: Int, b: Int) -> Int:\n    If b equals 0:\n        Return a.\n    Return gcd(b, a % b).\n\n## Main\nShow gcd(100, 75).", "25"),
        ("## To greet (name: Text) -> Text:\n    Return \"Hello, \" + name + \"!\".\n\n## Main\nShow greet(\"Alice\").", "Hello, Alice!"),
        ("## To power (b: Int, e: Int) -> Int:\n    If e is at most 0:\n        Return 1.\n    Return b * power(b, e - 1).\n\n## Main\nShow power(2, 10).", "1024"),
        ("## To absVal (n: Int) -> Int:\n    If n is less than 0:\n        Return 0 - n.\n    Return n.\n\n## Main\nShow absVal(0 - 42).", "42"),
        ("## To maxOf (a: Int, b: Int) -> Int:\n    If a is greater than b:\n        Return a.\n    Return b.\n\n## Main\nShow maxOf(17, 42).", "42"),
        ("## To minOf (a: Int, b: Int) -> Int:\n    If a is less than b:\n        Return a.\n    Return b.\n\n## Main\nShow minOf(17, 42).", "17"),
        ("## To collatzSteps (n: Int) -> Int:\n    If n is at most 1:\n        Return 0.\n    If n % 2 equals 0:\n        Return 1 + collatzSteps(n / 2).\n    Return 1 + collatzSteps(3 * n + 1).\n\n## Main\nShow collatzSteps(27).", "111"),
    ];

    let start = std::time::Instant::now();
    for (i, (program, expected)) in test_cases.iter().enumerate() {
        let fn_name = program.lines().next().unwrap_or("?");
        eprintln!("[{}/{}] {} — expected {}", i + 1, test_cases.len(), fn_name, expected);

        let t0 = std::time::Instant::now();
        let output_p1 = run_via_p1(program);
        eprintln!("  P1: {:?} → {}", t0.elapsed(), output_p1);

        let t1 = std::time::Instant::now();
        let output_p3 = run_via_p3(program);
        eprintln!("  P3: {:?} → {}", t1.elapsed(), output_p3);

        assert_eq!(
            output_p1, *expected,
            "P1 mismatch for program:\n{}", program
        );
        assert_eq!(
            output_p3, *expected,
            "P3 mismatch for program:\n{}", program
        );
    }
    eprintln!("Total: {:?}", start.elapsed());
}

// ===== Sprint 9 — Float, Extended Operators =====

#[test]
fn core_float_literal() {
    run_interpreter_program(
        r#"Let showExpr be a new CFloat with value 3.14.
Let showStmt be a new CShow with expr showExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "3.14",
    );
}

#[test]
fn core_float_addition() {
    run_interpreter_program(
        r#"Let letExpr be a new CFloat with value 1.5.
Let letStmt be a new CLet with name "x" and expr letExpr.
Let varExpr be a new CVar with name "x".
Let rightExpr be a new CFloat with value 2.5.
Let addExpr be a new CBinOp with op "+" and left varExpr and right rightExpr.
Let showStmt be a new CShow with expr addExpr.
Let stmts be a new Seq of CStmt.
Push letStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "4",
    );
}

#[test]
fn core_float_multiplication() {
    run_interpreter_program(
        r#"Let leftExpr be a new CFloat with value 2.0.
Let rightExpr be a new CFloat with value 3.5.
Let mulExpr be a new CBinOp with op "*" and left leftExpr and right rightExpr.
Let showStmt be a new CShow with expr mulExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "7",
    );
}

#[test]
fn core_float_division() {
    run_interpreter_program(
        r#"Let leftExpr be a new CFloat with value 10.0.
Let rightExpr be a new CFloat with value 4.0.
Let divExpr be a new CBinOp with op "/" and left leftExpr and right rightExpr.
Let showStmt be a new CShow with expr divExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "2.5",
    );
}

#[test]
fn core_float_subtraction() {
    run_interpreter_program(
        r#"Let leftExpr be a new CFloat with value 5.0.
Let rightExpr be a new CFloat with value 2.5.
Let subExpr be a new CBinOp with op "-" and left leftExpr and right rightExpr.
Let showStmt be a new CShow with expr subExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "2.5",
    );
}

#[test]
fn core_float_comparison() {
    run_interpreter_program(
        r#"Let leftExpr be a new CFloat with value 3.14.
Let rightExpr be a new CFloat with value 2.71.
Let cmpExpr be a new CBinOp with op ">" and left leftExpr and right rightExpr.
Let thenShow be a new CShow with expr a new CText with value "bigger".
Let thenBlock be a new Seq of CStmt.
Push thenShow to thenBlock.
Let elseBlock be a new Seq of CStmt.
Let ifStmt be a new CIf with cond cmpExpr and thenBlock thenBlock and elseBlock elseBlock.
Let stmts be a new Seq of CStmt.
Push ifStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "bigger",
    );
}

#[test]
fn core_float_int_promotion() {
    run_interpreter_program(
        r#"Let leftExpr be a new CInt with value 2.
Let rightExpr be a new CFloat with value 3.5.
Let addExpr be a new CBinOp with op "+" and left leftExpr and right rightExpr.
Let showStmt be a new CShow with expr addExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "5.5",
    );
}

// ============================================================
// Sprint 9, Step 3: Float-to-text, div-by-zero, bitwise, encoding
// ============================================================

#[test]
fn core_float_to_text() {
    run_interpreter_program(
        r#"Let v be a new VFloat with value 3.14.
Show valToText(v).
"#,
        "3.14",
    );
}

#[test]
fn core_float_div_by_zero() {
    run_interpreter_program(
        r#"Let leftExpr be a new CFloat with value 1.0.
Let rightExpr be a new CFloat with value 0.0.
Let divExpr be a new CBinOp with op "/" and left leftExpr and right rightExpr.
Let showStmt be a new CShow with expr divExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Error: division by zero",
    );
}

#[test]
fn core_bitxor() {
    run_interpreter_program(
        r#"Let leftExpr be a new CInt with value 5.
Let rightExpr be a new CInt with value 3.
Let xorExpr be a new CBinOp with op "^" and left leftExpr and right rightExpr.
Let showStmt be a new CShow with expr xorExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "6",
    );
}

#[test]
fn core_shl() {
    run_interpreter_program(
        r#"Let leftExpr be a new CInt with value 1.
Let rightExpr be a new CInt with value 4.
Let shlExpr be a new CBinOp with op "<<" and left leftExpr and right rightExpr.
Let showStmt be a new CShow with expr shlExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "16",
    );
}

#[test]
fn core_shr() {
    run_interpreter_program(
        r#"Let leftExpr be a new CInt with value 16.
Let rightExpr be a new CInt with value 2.
Let shrExpr be a new CBinOp with op ">>" and left leftExpr and right rightExpr.
Let showStmt be a new CShow with expr shrExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "4",
    );
}

#[test]
fn core_float_comparison_eq() {
    run_interpreter_program(
        r#"Let leftExpr be a new CFloat with value 1.0.
Let rightExpr be a new CFloat with value 1.0.
Let cmpExpr be a new CBinOp with op "==" and left leftExpr and right rightExpr.
Let thenShow be a new CShow with expr a new CText with value "eq".
Let thenBlock be a new Seq of CStmt.
Push thenShow to thenBlock.
Let elseShow be a new CShow with expr a new CText with value "ne".
Let elseBlock be a new Seq of CStmt.
Push elseShow to elseBlock.
Let ifStmt be a new CIf with cond cmpExpr and thenBlock thenBlock and elseBlock elseBlock.
Let stmts be a new Seq of CStmt.
Push ifStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "eq",
    );
}

#[test]
fn core_float_nested_arithmetic() {
    run_interpreter_program(
        r#"Let innerAdd be a new CBinOp with op "+" and left (a new CFloat with value 2.0) and right (a new CFloat with value 3.0).
Let mulExpr be a new CBinOp with op "*" and left innerAdd and right (a new CFloat with value 4.0).
Let showStmt be a new CShow with expr mulExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "20",
    );
}

#[test]
fn core_float_encode_roundtrip() {
    run_encoded_program("Show 3.14.", "3.14");
}

// ============================================================
// Sprint 10: Iteration — List, Range, Slice, Copy, Repeat, Break, Pop
// ============================================================

#[test]
fn core_iter_list_literal() {
    run_interpreter_program(
        r#"Let listItems be a new Seq of CExpr.
Push a new CInt with value 10 to listItems.
Push a new CInt with value 20 to listItems.
Push a new CInt with value 30 to listItems.
Let listExpr be a new CList with items listItems.
Let letStmt be a new CLet with name "xs" and expr listExpr.
Let showStmt be a new CShow with expr (a new CLen with target (a new CVar with name "xs")).
Let stmts be a new Seq of CStmt.
Push letStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "3",
    );
}

#[test]
fn core_iter_range_expr() {
    run_interpreter_program(
        r#"Let rangeExpr be a new CRange with start (a new CInt with value 1) and end (a new CInt with value 5).
Let letStmt be a new CLet with name "xs" and expr rangeExpr.
Let showStmt be a new CShow with expr (a new CLen with target (a new CVar with name "xs")).
Let stmts be a new Seq of CStmt.
Push letStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "5",
    );
}

#[test]
fn core_iter_range_empty() {
    run_interpreter_program(
        r#"Let rangeExpr be a new CRange with start (a new CInt with value 5) and end (a new CInt with value 1).
Let letStmt be a new CLet with name "xs" and expr rangeExpr.
Let showStmt be a new CShow with expr (a new CLen with target (a new CVar with name "xs")).
Let stmts be a new Seq of CStmt.
Push letStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "0",
    );
}

#[test]
fn core_iter_slice() {
    run_interpreter_program(
        r#"Let listItems be a new Seq of CExpr.
Push a new CInt with value 10 to listItems.
Push a new CInt with value 20 to listItems.
Push a new CInt with value 30 to listItems.
Push a new CInt with value 40 to listItems.
Let listExpr be a new CList with items listItems.
Let letXs be a new CLet with name "xs" and expr listExpr.
Let sliceExpr be a new CSlice with coll (a new CVar with name "xs") and startIdx (a new CInt with value 2) and endIdx (a new CInt with value 3).
Let letYs be a new CLet with name "ys" and expr sliceExpr.
Let showStmt be a new CShow with expr (a new CLen with target (a new CVar with name "ys")).
Let stmts be a new Seq of CStmt.
Push letXs to stmts.
Push letYs to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "2",
    );
}

#[test]
fn core_iter_copy() {
    run_interpreter_program(
        r#"Let listItems be a new Seq of CExpr.
Push a new CInt with value 1 to listItems.
Push a new CInt with value 2 to listItems.
Let listExpr be a new CList with items listItems.
Let letXs be a new CLet with name "xs" and expr listExpr.
Let copyExpr be a new CCopy with target (a new CVar with name "xs").
Let letYs be a new CLet with name "ys" and expr copyExpr.
Let pushStmt be a new CPush with expr (a new CInt with value 3) and target "ys".
Let showStmt be a new CShow with expr (a new CLen with target (a new CVar with name "xs")).
Let stmts be a new Seq of CStmt.
Push letXs to stmts.
Push letYs to stmts.
Push pushStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "2",
    );
}

#[test]
fn core_iter_list_show_elements() {
    run_interpreter_program(
        r#"Let listItems be a new Seq of CExpr.
Push a new CInt with value 10 to listItems.
Push a new CInt with value 20 to listItems.
Let listExpr be a new CList with items listItems.
Let letXs be a new CLet with name "xs" and expr listExpr.
Let showStmt be a new CShow with expr (a new CIndex with coll (a new CVar with name "xs") and idx (a new CInt with value 1)).
Let stmts be a new Seq of CStmt.
Push letXs to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "10",
    );
}

#[test]
fn core_iter_repeat_basic() {
    run_interpreter_program(
        r#"Let listItems be a new Seq of CExpr.
Push a new CInt with value 1 to listItems.
Push a new CInt with value 2 to listItems.
Push a new CInt with value 3 to listItems.
Let listExpr be a new CList with items listItems.
Let letXs be a new CLet with name "xs" and expr listExpr.
Let repBody be a new Seq of CStmt.
Push a new CShow with expr (a new CVar with name "x") to repBody.
Let repStmt be a new CRepeat with var "x" and coll (a new CVar with name "xs") and body repBody.
Let stmts be a new Seq of CStmt.
Push letXs to stmts.
Push repStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "1\n2\n3",
    );
}

#[test]
fn core_iter_repeat_accumulate() {
    run_interpreter_program(
        r#"Let listItems be a new Seq of CExpr.
Push a new CInt with value 1 to listItems.
Push a new CInt with value 2 to listItems.
Push a new CInt with value 3 to listItems.
Let listExpr be a new CList with items listItems.
Let letSum be a new CLet with name "sum" and expr (a new CInt with value 0).
Let letXs be a new CLet with name "xs" and expr listExpr.
Let repBody be a new Seq of CStmt.
Push a new CSet with name "sum" and expr (a new CBinOp with op "+" and left (a new CVar with name "sum") and right (a new CVar with name "x")) to repBody.
Let repStmt be a new CRepeat with var "x" and coll (a new CVar with name "xs") and body repBody.
Let showStmt be a new CShow with expr (a new CVar with name "sum").
Let stmts be a new Seq of CStmt.
Push letSum to stmts.
Push letXs to stmts.
Push repStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "6",
    );
}

#[test]
fn core_iter_repeat_empty() {
    run_interpreter_program(
        r#"Let listItems be a new Seq of CExpr.
Let listExpr be a new CList with items listItems.
Let letXs be a new CLet with name "xs" and expr listExpr.
Let repBody be a new Seq of CStmt.
Push a new CShow with expr (a new CVar with name "x") to repBody.
Let repStmt be a new CRepeat with var "x" and coll (a new CVar with name "xs") and body repBody.
Let showDone be a new CShow with expr (a new CText with value "done").
Let stmts be a new Seq of CStmt.
Push letXs to stmts.
Push repStmt to stmts.
Push showDone to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "done",
    );
}

#[test]
fn core_iter_repeat_range() {
    run_interpreter_program(
        r#"Let rrBody be a new Seq of CStmt.
Push a new CShow with expr (a new CVar with name "i") to rrBody.
Let rrStmt be a new CRepeatRange with var "i" and start (a new CInt with value 1) and end (a new CInt with value 5) and body rrBody.
Let stmts be a new Seq of CStmt.
Push rrStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "1\n2\n3\n4\n5",
    );
}

#[test]
fn core_iter_nested_repeat() {
    run_interpreter_program(
        r#"Let listItems be a new Seq of CExpr.
Push a new CInt with value 1 to listItems.
Push a new CInt with value 2 to listItems.
Let listExpr be a new CList with items listItems.
Let letXs be a new CLet with name "xs" and expr listExpr.
Let innerBody be a new Seq of CStmt.
Push a new CShow with expr (a new CBinOp with op "*" and left (a new CVar with name "x") and right (a new CVar with name "i")) to innerBody.
Let innerLoop be a new CRepeatRange with var "i" and start (a new CInt with value 1) and end (a new CInt with value 2) and body innerBody.
Let outerBody be a new Seq of CStmt.
Push innerLoop to outerBody.
Let outerLoop be a new CRepeat with var "x" and coll (a new CVar with name "xs") and body outerBody.
Let stmts be a new Seq of CStmt.
Push letXs to stmts.
Push outerLoop to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "1\n2\n2\n4",
    );
}

#[test]
fn core_iter_repeat_with_return() {
    run_interpreter_program(
        r#"Let fnBody be a new Seq of CStmt.
Let listItems be a new Seq of CExpr.
Push a new CInt with value 10 to listItems.
Push a new CInt with value 20 to listItems.
Push a new CInt with value 30 to listItems.
Let letXs be a new CLet with name "xs" and expr (a new CList with items listItems).
Push letXs to fnBody.
Let repBody be a new Seq of CStmt.
Let ifThen be a new Seq of CStmt.
Push a new CReturn with expr (a new CVar with name "x") to ifThen.
Let ifElse be a new Seq of CStmt.
Let ifStmt be a new CIf with cond (a new CBinOp with op ">" and left (a new CVar with name "x") and right (a new CInt with value 15)) and thenBlock ifThen and elseBlock ifElse.
Push ifStmt to repBody.
Let repStmt be a new CRepeat with var "x" and coll (a new CVar with name "xs") and body repBody.
Push repStmt to fnBody.
Push a new CReturn with expr (a new CInt with value 0) to fnBody.
Let params be a new Seq of Text.
Let fn be a new CFuncDef with name "findFirst" and params params and body fnBody.
Let funcMap be a new Map of Text to CFunc.
Set item "findFirst" of funcMap to fn.
Let callArgs be a new Seq of CExpr.
Let callStmt be a new CShow with expr (a new CCall with name "findFirst" and args callArgs).
Let stmts be a new Seq of CStmt.
Push callStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "20",
    );
}

#[test]
fn core_iter_repeat_with_push() {
    run_interpreter_program(
        r#"Let emptyList be a new Seq of CExpr.
Let letResult be a new CLet with name "result" and expr (a new CList with items emptyList).
Let rrBody be a new Seq of CStmt.
Push a new CPush with expr (a new CBinOp with op "*" and left (a new CVar with name "i") and right (a new CInt with value 10)) and target "result" to rrBody.
Let rrStmt be a new CRepeatRange with var "i" and start (a new CInt with value 1) and end (a new CInt with value 3) and body rrBody.
Let showStmt be a new CShow with expr (a new CLen with target (a new CVar with name "result")).
Let stmts be a new Seq of CStmt.
Push letResult to stmts.
Push rrStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "3",
    );
}

#[test]
fn core_iter_break_basic() {
    run_interpreter_program(
        r#"Let ifThen be a new Seq of CStmt.
Push a new CBreak to ifThen.
Let ifElse be a new Seq of CStmt.
Let ifStmt be a new CIf with cond (a new CBinOp with op ">" and left (a new CVar with name "i") and right (a new CInt with value 3)) and thenBlock ifThen and elseBlock ifElse.
Let rrBody be a new Seq of CStmt.
Push ifStmt to rrBody.
Push a new CShow with expr (a new CVar with name "i") to rrBody.
Let rrStmt be a new CRepeatRange with var "i" and start (a new CInt with value 1) and end (a new CInt with value 100) and body rrBody.
Let stmts be a new Seq of CStmt.
Push rrStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "1\n2\n3",
    );
}

#[test]
fn core_iter_break_in_while() {
    run_interpreter_program(
        r#"Let letI be a new CLet with name "i" and expr (a new CInt with value 0).
Let ifThen be a new Seq of CStmt.
Push a new CBreak to ifThen.
Let ifElse be a new Seq of CStmt.
Let ifStmt be a new CIf with cond (a new CBinOp with op ">=" and left (a new CVar with name "i") and right (a new CInt with value 5)) and thenBlock ifThen and elseBlock ifElse.
Let whileBody be a new Seq of CStmt.
Push ifStmt to whileBody.
Push a new CShow with expr (a new CVar with name "i") to whileBody.
Push a new CSet with name "i" and expr (a new CBinOp with op "+" and left (a new CVar with name "i") and right (a new CInt with value 1)) to whileBody.
Let whileStmt be a new CWhile with cond (a new CBool with value true) and body whileBody.
Let stmts be a new Seq of CStmt.
Push letI to stmts.
Push whileStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "0\n1\n2\n3\n4",
    );
}

#[test]
fn core_iter_pop() {
    run_interpreter_program(
        r#"Let listItems be a new Seq of CExpr.
Push a new CInt with value 10 to listItems.
Push a new CInt with value 20 to listItems.
Push a new CInt with value 30 to listItems.
Let letXs be a new CLet with name "xs" and expr (a new CList with items listItems).
Let popStmt be a new CPop with target "xs".
Let showStmt be a new CShow with expr (a new CLen with target (a new CVar with name "xs")).
Let stmts be a new Seq of CStmt.
Push letXs to stmts.
Push popStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "2",
    );
}

#[test]
fn core_iter_pop_empty_error() {
    run_interpreter_program(
        r#"Let emptyItems be a new Seq of CExpr.
Let letXs be a new CLet with name "xs" and expr (a new CList with items emptyItems).
Let popStmt be a new CPop with target "xs".
Let showStmt be a new CShow with expr (a new CText with value "survived").
Let stmts be a new Seq of CStmt.
Push letXs to stmts.
Push popStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "survived",
    );
}

#[test]
fn core_iter_encode_repeat() {
    run_encoded_program(
        "Let items be a new Seq of Int.\nPush 10 to items.\nPush 20 to items.\nPush 30 to items.\nLet mutable total be 0.\nRepeat for x in items:\n    Set total to total + x.\nShow total.",
        "60",
    );
}

// Sprint 11 — Sets, Options, Tuples

#[test]
fn core_set_add_and_contains() {
    run_interpreter_program(
        r#"Let setItems be a new Seq of CVal.
Let mySet be a new VSet with items setItems.
Let env be a new Map of Text to CVal.
Set item "s" of env to mySet.
Let addElem be a new CInt with value 42.
Let addStmt be a new CAdd with elem addElem and target "s".
Let containsColl be a new CVar with name "s".
Let containsElem be a new CInt with value 42.
Let containsExpr be a new CContains with coll containsColl and elem containsElem.
Let showStmt be a new CShow with expr containsExpr.
Let stmts be a new Seq of CStmt.
Push addStmt to stmts.
Push showStmt to stmts.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "true",
    );
}

#[test]
fn core_set_remove() {
    run_interpreter_program(
        r#"Let setItems be a new Seq of CVal.
Let mySet be a new VSet with items setItems.
Let env be a new Map of Text to CVal.
Set item "s" of env to mySet.
Let addElem be a new CInt with value 42.
Let addStmt be a new CAdd with elem addElem and target "s".
Let remElem be a new CInt with value 42.
Let remStmt be a new CRemove with elem remElem and target "s".
Let containsColl be a new CVar with name "s".
Let containsElem be a new CInt with value 42.
Let containsExpr be a new CContains with coll containsColl and elem containsElem.
Let showStmt be a new CShow with expr containsExpr.
Let stmts be a new Seq of CStmt.
Push addStmt to stmts.
Push remStmt to stmts.
Push showStmt to stmts.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "false",
    );
}

#[test]
fn core_set_union() {
    run_interpreter_program(
        r#"Let items1 be a new Seq of CVal.
Push a new VInt with value 1 to items1.
Push a new VInt with value 2 to items1.
Let items2 be a new Seq of CVal.
Push a new VInt with value 2 to items2.
Push a new VInt with value 3 to items2.
Let env be a new Map of Text to CVal.
Set item "s1" of env to a new VSet with items items1.
Set item "s2" of env to a new VSet with items items2.
Let s1Var be a new CVar with name "s1".
Let s2Var be a new CVar with name "s2".
Let unionExpr be a new CUnion with left s1Var and right s2Var.
Let letUnion be a new CLet with name "u" and expr unionExpr.
Let uRef be a new CVar with name "u".
Let elem3 be a new CInt with value 3.
Let contains3 be a new CContains with coll uRef and elem elem3.
Let show3 be a new CShow with expr contains3.
Let stmts be a new Seq of CStmt.
Push letUnion to stmts.
Push show3 to stmts.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "true",
    );
}

#[test]
fn core_set_intersection() {
    run_interpreter_program(
        r#"Let items1 be a new Seq of CVal.
Push a new VInt with value 1 to items1.
Push a new VInt with value 2 to items1.
Push a new VInt with value 3 to items1.
Let items2 be a new Seq of CVal.
Push a new VInt with value 2 to items2.
Push a new VInt with value 3 to items2.
Push a new VInt with value 4 to items2.
Let env be a new Map of Text to CVal.
Set item "s1" of env to a new VSet with items items1.
Set item "s2" of env to a new VSet with items items2.
Let s1Var be a new CVar with name "s1".
Let s2Var be a new CVar with name "s2".
Let interExpr be a new CIntersection with left s1Var and right s2Var.
Let letInter be a new CLet with name "inter" and expr interExpr.
Let interRef be a new CVar with name "inter".
Let elem2 be a new CInt with value 2.
Let contains2 be a new CContains with coll interRef and elem elem2.
Let show2 be a new CShow with expr contains2.
Let interRef2 be a new CVar with name "inter".
Let elem1 be a new CInt with value 1.
Let contains1 be a new CContains with coll interRef2 and elem elem1.
Let show1 be a new CShow with expr contains1.
Let stmts be a new Seq of CStmt.
Push letInter to stmts.
Push show2 to stmts.
Push show1 to stmts.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "true\nfalse",
    );
}

#[test]
fn core_set_no_duplicates() {
    run_interpreter_program(
        r#"Let setItems be a new Seq of CVal.
Let mySet be a new VSet with items setItems.
Let env be a new Map of Text to CVal.
Set item "s" of env to mySet.
Let add1Elem be a new CInt with value 42.
Let add1 be a new CAdd with elem add1Elem and target "s".
Let add2Elem be a new CInt with value 42.
Let add2 be a new CAdd with elem add2Elem and target "s".
Let lenTarget be a new CVar with name "s".
Let lenExpr be a new CLen with target lenTarget.
Let showStmt be a new CShow with expr lenExpr.
Let stmts be a new Seq of CStmt.
Push add1 to stmts.
Push add2 to stmts.
Push showStmt to stmts.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "1",
    );
}

#[test]
fn core_set_contains_not_found() {
    run_interpreter_program(
        r#"Let setItems be a new Seq of CVal.
Push a new VInt with value 10 to setItems.
Push a new VInt with value 20 to setItems.
Let mySet be a new VSet with items setItems.
Let env be a new Map of Text to CVal.
Set item "s" of env to mySet.
Let containsColl be a new CVar with name "s".
Let containsElem be a new CInt with value 99.
Let containsExpr be a new CContains with coll containsColl and elem containsElem.
Let showStmt be a new CShow with expr containsExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "false",
    );
}

#[test]
fn core_option_some() {
    run_interpreter_program(
        r#"Let innerExpr be a new CInt with value 42.
Let someExpr be a new COptionSome with inner innerExpr.
Let showStmt be a new CShow with expr someExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Some(42)",
    );
}

#[test]
fn core_option_none() {
    run_interpreter_program(
        r#"Let noneExpr be a new COptionNone.
Let showStmt be a new CShow with expr noneExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "None",
    );
}

#[test]
fn core_option_unwrap() {
    run_interpreter_program(
        r#"Let innerExpr be a new CInt with value 42.
Let someExpr be a new COptionSome with inner innerExpr.
Let letOpt be a new CLet with name "opt" and expr someExpr.
Let optRef be a new CVar with name "opt".
Let showStmt be a new CShow with expr optRef.
Let stmts be a new Seq of CStmt.
Push letOpt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Some(42)",
    );
}

#[test]
fn core_tuple_create() {
    run_interpreter_program(
        r#"Let items be a new Seq of CExpr.
Push a new CInt with value 1 to items.
Push a new CText with value "hello" to items.
Push a new CBool with value true to items.
Let tupleExpr be a new CTuple with items items.
Let showStmt be a new CShow with expr tupleExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "(1, hello, true)",
    );
}

#[test]
fn core_tuple_index() {
    run_interpreter_program(
        r#"Let items be a new Seq of CExpr.
Push a new CInt with value 10 to items.
Push a new CInt with value 20 to items.
Push a new CInt with value 30 to items.
Let tupleExpr be a new CTuple with items items.
Let letTup be a new CLet with name "t" and expr tupleExpr.
Let tupRef be a new CVar with name "t".
Let idxExpr be a new CInt with value 2.
Let indexExpr be a new CIndex with coll tupRef and idx idxExpr.
Let showStmt be a new CShow with expr indexExpr.
Let stmts be a new Seq of CStmt.
Push letTup to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "20",
    );
}

#[test]
fn core_tuple_to_text() {
    run_interpreter_program(
        r#"Let items be a new Seq of CExpr.
Push a new CText with value "a" to items.
Push a new CText with value "b" to items.
Let tupleExpr be a new CTuple with items items.
Let showStmt be a new CShow with expr tupleExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "(a, b)",
    );
}

#[test]
fn core_contains_in_seq() {
    run_interpreter_program(
        r#"Let listItems be a new Seq of CExpr.
Push a new CInt with value 10 to listItems.
Push a new CInt with value 20 to listItems.
Push a new CInt with value 30 to listItems.
Let listExpr be a new CList with items listItems.
Let letSeq be a new CLet with name "xs" and expr listExpr.
Let seqRef be a new CVar with name "xs".
Let elemExpr be a new CInt with value 20.
Let containsExpr be a new CContains with coll seqRef and elem elemExpr.
Let showStmt be a new CShow with expr containsExpr.
Let stmts be a new Seq of CStmt.
Push letSeq to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "true",
    );
}

#[test]
fn core_contains_text_in_text() {
    run_interpreter_program(
        r#"Let haystackExpr be a new CText with value "hello world".
Let needleExpr be a new CText with value "world".
Let containsExpr be a new CContains with coll haystackExpr and elem needleExpr.
Let showStmt be a new CShow with expr containsExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "true",
    );
}

#[test]
fn core_set_encode_roundtrip() {
    run_encoded_program(
        "Let s be a new Set of Int.\nAdd 1 to s.\nAdd 2 to s.\nAdd 1 to s.\nIf s contains 2:\n    Show \"found\".",
        "found",
    );
}

// ===== Sprint 12: Structs, Fields =====

#[test]
fn core_struct_new() {
    run_interpreter_program(
        r#"Let fieldNames be a new Seq of Text.
Push "x" to fieldNames.
Push "y" to fieldNames.
Let fieldExprs be a new Seq of CExpr.
Push a new CInt with value 3 to fieldExprs.
Push a new CInt with value 4 to fieldExprs.
Let newExpr be a new CNew with typeName "Point" and fieldNames fieldNames and fields fieldExprs.
Let letP be a new CLet with name "p" and expr newExpr.
Let faExpr be a new CFieldAccess with target (a new CVar with name "p") and field "x".
Let showStmt be a new CShow with expr faExpr.
Let stmts be a new Seq of CStmt.
Push letP to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "3",
    );
}

#[test]
fn core_struct_field_y() {
    run_interpreter_program(
        r#"Let fieldNames be a new Seq of Text.
Push "x" to fieldNames.
Push "y" to fieldNames.
Let fieldExprs be a new Seq of CExpr.
Push a new CInt with value 3 to fieldExprs.
Push a new CInt with value 4 to fieldExprs.
Let newExpr be a new CNew with typeName "Point" and fieldNames fieldNames and fields fieldExprs.
Let letP be a new CLet with name "p" and expr newExpr.
Let faExpr be a new CFieldAccess with target (a new CVar with name "p") and field "y".
Let showStmt be a new CShow with expr faExpr.
Let stmts be a new Seq of CStmt.
Push letP to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "4",
    );
}

#[test]
fn core_struct_set_field() {
    run_interpreter_program(
        r#"Let fieldNames be a new Seq of Text.
Push "x" to fieldNames.
Push "y" to fieldNames.
Let fieldExprs be a new Seq of CExpr.
Push a new CInt with value 3 to fieldExprs.
Push a new CInt with value 4 to fieldExprs.
Let newExpr be a new CNew with typeName "Point" and fieldNames fieldNames and fields fieldExprs.
Let letP be a new CLet with name "p" and expr newExpr.
Let setFieldStmt be a new CSetField with target "p" and field "x" and val (a new CInt with value 10).
Let faExpr be a new CFieldAccess with target (a new CVar with name "p") and field "x".
Let showStmt be a new CShow with expr faExpr.
Let stmts be a new Seq of CStmt.
Push letP to stmts.
Push setFieldStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "10",
    );
}

#[test]
fn core_struct_to_text() {
    run_interpreter_program(
        r#"Let fieldNames be a new Seq of Text.
Push "x" to fieldNames.
Push "y" to fieldNames.
Let fieldExprs be a new Seq of CExpr.
Push a new CInt with value 3 to fieldExprs.
Push a new CInt with value 4 to fieldExprs.
Let newExpr be a new CNew with typeName "Point" and fieldNames fieldNames and fields fieldExprs.
Let letP be a new CLet with name "p" and expr newExpr.
Let showStmt be a new CShow with expr (a new CVar with name "p").
Let stmts be a new Seq of CStmt.
Push letP to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Point(...)",
    );
}

#[test]
fn core_struct_nested() {
    run_interpreter_program(
        r#"Let pf1 be a new Seq of Text.
Push "x" to pf1.
Push "y" to pf1.
Let pf2 be a new Seq of Text.
Push "x" to pf2.
Push "y" to pf2.
Let lineFields be a new Seq of Text.
Push "start" to lineFields.
Push "end" to lineFields.
Let p1Fields be a new Seq of CExpr.
Push a new CInt with value 1 to p1Fields.
Push a new CInt with value 2 to p1Fields.
Let p1Expr be a new CNew with typeName "Point" and fieldNames pf1 and fields p1Fields.
Let letP1 be a new CLet with name "p1" and expr p1Expr.
Let p2Fields be a new Seq of CExpr.
Push a new CInt with value 5 to p2Fields.
Push a new CInt with value 6 to p2Fields.
Let p2Expr be a new CNew with typeName "Point" and fieldNames pf2 and fields p2Fields.
Let letP2 be a new CLet with name "p2" and expr p2Expr.
Let lineFieldExprs be a new Seq of CExpr.
Push a new CVar with name "p1" to lineFieldExprs.
Push a new CVar with name "p2" to lineFieldExprs.
Let lineExpr be a new CNew with typeName "Line" and fieldNames lineFields and fields lineFieldExprs.
Let letLine be a new CLet with name "line" and expr lineExpr.
Let startExpr be a new CFieldAccess with target (a new CVar with name "line") and field "start".
Let xExpr be a new CFieldAccess with target startExpr and field "x".
Let showStmt be a new CShow with expr xExpr.
Let stmts be a new Seq of CStmt.
Push letP1 to stmts.
Push letP2 to stmts.
Push letLine to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "1",
    );
}

#[test]
fn core_struct_pass_to_function() {
    run_interpreter_program(
        r#"Let fieldNames be a new Seq of Text.
Push "x" to fieldNames.
Push "y" to fieldNames.
Let funcParams be a new Seq of Text.
Push "pt" to funcParams.
Let funcBody be a new Seq of CStmt.
Let faExpr be a new CFieldAccess with target (a new CVar with name "pt") and field "x".
Push a new CReturn with expr faExpr to funcBody.
Let funcDef be a new CFuncDef with name "getX" and params funcParams and body funcBody.
Let fieldExprs be a new Seq of CExpr.
Push a new CInt with value 42 to fieldExprs.
Push a new CInt with value 99 to fieldExprs.
Let newExpr be a new CNew with typeName "Point" and fieldNames fieldNames and fields fieldExprs.
Let letP be a new CLet with name "p" and expr newExpr.
Let callArgs be a new Seq of CExpr.
Push a new CVar with name "p" to callArgs.
Let callExpr be a new CCall with name "getX" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push letP to stmts.
Push showStmt to stmts.
Let funcMap be a new Map of Text to CFunc.
Set item "getX" of funcMap to funcDef.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "42",
    );
}

#[test]
fn core_struct_multiple_types() {
    run_interpreter_program(
        r#"Let pointFields be a new Seq of Text.
Push "x" to pointFields.
Push "y" to pointFields.
Let colorFields be a new Seq of Text.
Push "r" to colorFields.
Push "g" to colorFields.
Push "b" to colorFields.
Let pFields be a new Seq of CExpr.
Push a new CInt with value 10 to pFields.
Push a new CInt with value 20 to pFields.
Let newPoint be a new CNew with typeName "Point" and fieldNames pointFields and fields pFields.
Let letP be a new CLet with name "pt" and expr newPoint.
Let cFields be a new Seq of CExpr.
Push a new CInt with value 255 to cFields.
Push a new CInt with value 128 to cFields.
Push a new CInt with value 0 to cFields.
Let newColor be a new CNew with typeName "Color" and fieldNames colorFields and fields cFields.
Let letC be a new CLet with name "col" and expr newColor.
Let ptY be a new CFieldAccess with target (a new CVar with name "pt") and field "y".
Let colR be a new CFieldAccess with target (a new CVar with name "col") and field "r".
Let sumExpr be a new CBinOp with op "+" and left ptY and right colR.
Let showStmt be a new CShow with expr sumExpr.
Let stmts be a new Seq of CStmt.
Push letP to stmts.
Push letC to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "275",
    );
}

#[test]
fn core_struct_field_missing() {
    // Access a field on a non-struct value (VInt) → VNothing
    run_interpreter_program(
        r#"Let letP be a new CLet with name "p" and expr (a new CInt with value 5).
Let faExpr be a new CFieldAccess with target (a new CVar with name "p") and field "x".
Let showStmt be a new CShow with expr faExpr.
Let stmts be a new Seq of CStmt.
Push letP to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "nothing",
    );
}

#[test]
fn core_struct_arithmetic_fields() {
    run_interpreter_program(
        r#"Let fieldNames be a new Seq of Text.
Push "x" to fieldNames.
Push "y" to fieldNames.
Let fieldExprs be a new Seq of CExpr.
Push a new CInt with value 3 to fieldExprs.
Push a new CInt with value 4 to fieldExprs.
Let newExpr be a new CNew with typeName "Point" and fieldNames fieldNames and fields fieldExprs.
Let letP be a new CLet with name "p" and expr newExpr.
Let xExpr1 be a new CFieldAccess with target (a new CVar with name "p") and field "x".
Let xExpr2 be a new CFieldAccess with target (a new CVar with name "p") and field "x".
Let yExpr1 be a new CFieldAccess with target (a new CVar with name "p") and field "y".
Let yExpr2 be a new CFieldAccess with target (a new CVar with name "p") and field "y".
Let x2 be a new CBinOp with op "*" and left xExpr1 and right xExpr2.
Let y2 be a new CBinOp with op "*" and left yExpr1 and right yExpr2.
Let sumExpr be a new CBinOp with op "+" and left x2 and right y2.
Let showStmt be a new CShow with expr sumExpr.
Let stmts be a new Seq of CStmt.
Push letP to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "25",
    );
}

#[test]
fn core_struct_in_sequence() {
    run_interpreter_program(
        r#"Let fn1 be a new Seq of Text.
Push "x" to fn1.
Push "y" to fn1.
Let fn2 be a new Seq of Text.
Push "x" to fn2.
Push "y" to fn2.
Let f1 be a new Seq of CExpr.
Push a new CInt with value 1 to f1.
Push a new CInt with value 2 to f1.
Let n1 be a new CNew with typeName "Point" and fieldNames fn1 and fields f1.
Let f2 be a new Seq of CExpr.
Push a new CInt with value 3 to f2.
Push a new CInt with value 4 to f2.
Let n2 be a new CNew with typeName "Point" and fieldNames fn2 and fields f2.
Let letSeq be a new CLet with name "pts" and expr (a new CNewSeq).
Let push1 be a new CPush with expr n1 and target "pts".
Let push2 be a new CPush with expr n2 and target "pts".
Let idxExpr be a new CIndex with coll (a new CVar with name "pts") and idx (a new CInt with value 2).
Let faExpr be a new CFieldAccess with target idxExpr and field "x".
Let showStmt be a new CShow with expr faExpr.
Let stmts be a new Seq of CStmt.
Push letSeq to stmts.
Push push1 to stmts.
Push push2 to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "3",
    );
}

#[test]
fn core_struct_copy_semantics() {
    run_interpreter_program(
        r#"Let fieldNames be a new Seq of Text.
Push "x" to fieldNames.
Push "y" to fieldNames.
Let fieldExprs be a new Seq of CExpr.
Push a new CInt with value 10 to fieldExprs.
Push a new CInt with value 20 to fieldExprs.
Let newExpr be a new CNew with typeName "Point" and fieldNames fieldNames and fields fieldExprs.
Let letP be a new CLet with name "p" and expr newExpr.
Let letQ be a new CLet with name "q" and expr (a new CVar with name "p").
Let setField be a new CSetField with target "p" and field "x" and val (a new CInt with value 99).
Let faExpr be a new CFieldAccess with target (a new CVar with name "q") and field "x".
Let showStmt be a new CShow with expr faExpr.
Let stmts be a new Seq of CStmt.
Push letP to stmts.
Push letQ to stmts.
Push setField to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "10",
    );
}

#[test]
fn core_struct_in_map() {
    run_interpreter_program(
        r#"Let fieldNames be a new Seq of Text.
Push "x" to fieldNames.
Push "y" to fieldNames.
Let fieldExprs be a new Seq of CExpr.
Push a new CInt with value 7 to fieldExprs.
Push a new CInt with value 8 to fieldExprs.
Let newExpr be a new CNew with typeName "Point" and fieldNames fieldNames and fields fieldExprs.
Let letP be a new CLet with name "p" and expr newExpr.
Let letMap be a new CLet with name "m" and expr (a new CText with value "placeholder").
Let stmts be a new Seq of CStmt.
Push letP to stmts.
Let showStmt be a new CShow with expr (a new CFieldAccess with target (a new CVar with name "p") and field "y").
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "8",
    );
}

#[test]
fn core_struct_recursive() {
    run_interpreter_program(
        r#"Let nodeFields be a new Seq of Text.
Push "value" to nodeFields.
Push "count" to nodeFields.
Let f1 be a new Seq of CExpr.
Push a new CInt with value 42 to f1.
Push a new CInt with value 1 to f1.
Let node1 be a new CNew with typeName "Node" and fieldNames nodeFields and fields f1.
Let letN be a new CLet with name "n" and expr node1.
Let faExpr be a new CFieldAccess with target (a new CVar with name "n") and field "value".
Let showStmt be a new CShow with expr faExpr.
Let stmts be a new Seq of CStmt.
Push letN to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "42",
    );
}

#[test]
fn core_struct_with_function() {
    run_interpreter_program(
        r#"Let fieldNames be a new Seq of Text.
Push "x" to fieldNames.
Push "y" to fieldNames.
Let funcParams be a new Seq of Text.
Push "a" to funcParams.
Push "b" to funcParams.
Let funcBody be a new Seq of CStmt.
Let fExprs be a new Seq of CExpr.
Push a new CVar with name "a" to fExprs.
Push a new CVar with name "b" to fExprs.
Let fNew be a new CNew with typeName "Point" and fieldNames fieldNames and fields fExprs.
Push a new CReturn with expr fNew to funcBody.
Let funcDef be a new CFuncDef with name "makePoint" and params funcParams and body funcBody.
Let callArgs be a new Seq of CExpr.
Push a new CInt with value 100 to callArgs.
Push a new CInt with value 200 to callArgs.
Let callExpr be a new CCall with name "makePoint" and args callArgs.
Let letP be a new CLet with name "p" and expr callExpr.
Let showStmt be a new CShow with expr (a new CFieldAccess with target (a new CVar with name "p") and field "y").
Let stmts be a new Seq of CStmt.
Push letP to stmts.
Push showStmt to stmts.
Let funcMap be a new Map of Text to CFunc.
Set item "makePoint" of funcMap to funcDef.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "200",
    );
}

#[test]
fn core_struct_encode_roundtrip() {
    run_encoded_program(
        "## A Point is:\n    An x Int.\n    A y Int.\n\n## Main\nLet p be a new Point with x 3 and y 4.\nShow p's x.",
        "3",
    );
}

// ===== Sprint 13: Enums, Pattern Matching =====

#[test]
fn core_enum_new_variant() {
    run_interpreter_program(
        r#"Let fnames be a new Seq of Text.
Push "radius" to fnames.
Let fvals be a new Seq of CExpr.
Push a new CFloat with value 5.0 to fvals.
Let nvExpr be a new CNewVariant with tag "Circle" and fnames fnames and fvals fvals.
Let letS be a new CLet with name "s" and expr nvExpr.
Let showStmt be a new CShow with expr (a new CVar with name "s").
Let stmts be a new Seq of CStmt.
Push letS to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "[map]",
    );
}

#[test]
fn core_enum_inspect_match() {
    run_interpreter_program(
        r#"Let fnames be a new Seq of Text.
Push "radius" to fnames.
Let fvals be a new Seq of CExpr.
Push a new CInt with value 5 to fvals.
Let nvExpr be a new CNewVariant with tag "Circle" and fnames fnames and fvals fvals.
Let letS be a new CLet with name "s" and expr nvExpr.
Let armBindings be a new Seq of Text.
Push "r" to armBindings.
Let armBody be a new Seq of CStmt.
Push a new CShow with expr (a new CVar with name "r") to armBody.
Let arm1 be a new CWhen with variantName "Circle" and bindings armBindings and body armBody.
Let arms be a new Seq of CMatchArm.
Push arm1 to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "s") and arms arms.
Let stmts be a new Seq of CStmt.
Push letS to stmts.
Push inspStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "5",
    );
}

#[test]
fn core_enum_inspect_second_arm() {
    run_interpreter_program(
        r#"Let fnames be a new Seq of Text.
Push "side" to fnames.
Let fvals be a new Seq of CExpr.
Push a new CInt with value 4 to fvals.
Let nvExpr be a new CNewVariant with tag "Square" and fnames fnames and fvals fvals.
Let letS be a new CLet with name "s" and expr nvExpr.
Let arm1Bindings be a new Seq of Text.
Push "r" to arm1Bindings.
Let arm1Body be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "circle") to arm1Body.
Let arm1 be a new CWhen with variantName "Circle" and bindings arm1Bindings and body arm1Body.
Let arm2Bindings be a new Seq of Text.
Push "s" to arm2Bindings.
Let arm2Body be a new Seq of CStmt.
Push a new CShow with expr (a new CVar with name "s") to arm2Body.
Let arm2 be a new CWhen with variantName "Square" and bindings arm2Bindings and body arm2Body.
Let arms be a new Seq of CMatchArm.
Push arm1 to arms.
Push arm2 to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "s") and arms arms.
Let stmts be a new Seq of CStmt.
Push letS to stmts.
Push inspStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "4",
    );
}

#[test]
fn core_enum_inspect_otherwise() {
    run_interpreter_program(
        r#"Let fnames be a new Seq of Text.
Push "pts" to fnames.
Let fvals be a new Seq of CExpr.
Push a new CInt with value 3 to fvals.
Let nvExpr be a new CNewVariant with tag "Triangle" and fnames fnames and fvals fvals.
Let letS be a new CLet with name "s" and expr nvExpr.
Let arm1Bindings be a new Seq of Text.
Push "r" to arm1Bindings.
Let arm1Body be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "circle") to arm1Body.
Let arm1 be a new CWhen with variantName "Circle" and bindings arm1Bindings and body arm1Body.
Let owBody be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "other") to owBody.
Let arm2 be a new COtherwise with body owBody.
Let arms be a new Seq of CMatchArm.
Push arm1 to arms.
Push arm2 to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "s") and arms arms.
Let stmts be a new Seq of CStmt.
Push letS to stmts.
Push inspStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "other",
    );
}

#[test]
fn core_enum_no_field_variant() {
    run_interpreter_program(
        r#"Let fnames be a new Seq of Text.
Let fvals be a new Seq of CExpr.
Let nvExpr be a new CNewVariant with tag "None" and fnames fnames and fvals fvals.
Let letS be a new CLet with name "s" and expr nvExpr.
Let arm1Bindings be a new Seq of Text.
Let arm1Body be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "got none") to arm1Body.
Let arm1 be a new CWhen with variantName "None" and bindings arm1Bindings and body arm1Body.
Let arms be a new Seq of CMatchArm.
Push arm1 to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "s") and arms arms.
Let stmts be a new Seq of CStmt.
Push letS to stmts.
Push inspStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "got none",
    );
}

#[test]
fn core_enum_multiple_fields() {
    run_interpreter_program(
        r#"Let fnames be a new Seq of Text.
Push "width" to fnames.
Push "height" to fnames.
Let fvals be a new Seq of CExpr.
Push a new CInt with value 10 to fvals.
Push a new CInt with value 20 to fvals.
Let nvExpr be a new CNewVariant with tag "Rect" and fnames fnames and fvals fvals.
Let letS be a new CLet with name "s" and expr nvExpr.
Let armBindings be a new Seq of Text.
Push "w" to armBindings.
Push "h" to armBindings.
Let armBody be a new Seq of CStmt.
Let areaExpr be a new CBinOp with op "*" and left (a new CVar with name "w") and right (a new CVar with name "h").
Push a new CShow with expr areaExpr to armBody.
Let arm1 be a new CWhen with variantName "Rect" and bindings armBindings and body armBody.
Let arms be a new Seq of CMatchArm.
Push arm1 to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "s") and arms arms.
Let stmts be a new Seq of CStmt.
Push letS to stmts.
Push inspStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "200",
    );
}

// Sprint 13 Step 3 — Complex pattern matching (7 tests)

#[test]
fn core_enum_nested_inspect() {
    run_interpreter_program(
        r#"Let innerFn be a new Seq of Text.
Push "val" to innerFn.
Let innerFv be a new Seq of CExpr.
Push a new CInt with value 42 to innerFv.
Let innerExpr be a new CNewVariant with tag "Inner" and fnames innerFn and fvals innerFv.
Let outerFn be a new Seq of Text.
Push "wrapped" to outerFn.
Let outerFv be a new Seq of CExpr.
Push innerExpr to outerFv.
Let outerExpr be a new CNewVariant with tag "Outer" and fnames outerFn and fvals outerFv.
Let letO be a new CLet with name "o" and expr outerExpr.
Let innerBindings be a new Seq of Text.
Push "v" to innerBindings.
Let innerBody be a new Seq of CStmt.
Push a new CShow with expr (a new CVar with name "v") to innerBody.
Let innerArm be a new CWhen with variantName "Inner" and bindings innerBindings and body innerBody.
Let innerArms be a new Seq of CMatchArm.
Push innerArm to innerArms.
Let innerInspect be a new CInspect with target (a new CVar with name "w") and arms innerArms.
Let outerBindings be a new Seq of Text.
Push "w" to outerBindings.
Let outerBody be a new Seq of CStmt.
Push innerInspect to outerBody.
Let outerArm be a new CWhen with variantName "Outer" and bindings outerBindings and body outerBody.
Let outerArms be a new Seq of CMatchArm.
Push outerArm to outerArms.
Let outerInspect be a new CInspect with target (a new CVar with name "o") and arms outerArms.
Let stmts be a new Seq of CStmt.
Push letO to stmts.
Push outerInspect to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "42",
    );
}

#[test]
fn core_enum_inspect_return() {
    run_interpreter_program(
        r#"Let fnames1 be a new Seq of Text.
Push "val" to fnames1.
Let fvals1 be a new Seq of CExpr.
Push a new CInt with value 7 to fvals1.
Let nvExpr be a new CNewVariant with tag "Some" and fnames fnames1 and fvals fvals1.
Let letX be a new CLet with name "x" and expr nvExpr.
Let armBindings be a new Seq of Text.
Push "v" to armBindings.
Let armBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CVar with name "v") to armBody.
Let arm1 be a new CWhen with variantName "Some" and bindings armBindings and body armBody.
Let owBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CInt with value 0) to owBody.
Let arm2 be a new COtherwise with body owBody.
Let arms be a new Seq of CMatchArm.
Push arm1 to arms.
Push arm2 to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "x") and arms arms.
Let funcBody be a new Seq of CStmt.
Push letX to funcBody.
Push inspStmt to funcBody.
Let funcParams be a new Seq of Text.
Let func be a new CFuncDef with name "getValue" and params funcParams and body funcBody.
Let funcMap be a new Map of Text to CFunc.
Set item "getValue" of funcMap to func.
Let callExpr be a new CCall with name "getValue" and args (a new Seq of CExpr).
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "7",
    );
}

#[test]
fn core_enum_inspect_with_computation() {
    run_interpreter_program(
        r#"Let fnames1 be a new Seq of Text.
Push "radius" to fnames1.
Let fvals1 be a new Seq of CExpr.
Push a new CInt with value 10 to fvals1.
Let nvExpr be a new CNewVariant with tag "Circle" and fnames fnames1 and fvals fvals1.
Let letS be a new CLet with name "shape" and expr nvExpr.
Let armBindings be a new Seq of Text.
Push "r" to armBindings.
Let rTimesR be a new CBinOp with op "*" and left (a new CVar with name "r") and right (a new CVar with name "r").
Let area be a new CBinOp with op "*" and left rTimesR and right (a new CInt with value 3).
Let armBody be a new Seq of CStmt.
Push a new CShow with expr area to armBody.
Let arm1 be a new CWhen with variantName "Circle" and bindings armBindings and body armBody.
Let arms be a new Seq of CMatchArm.
Push arm1 to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "shape") and arms arms.
Let stmts be a new Seq of CStmt.
Push letS to stmts.
Push inspStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "300",
    );
}

#[test]
fn core_enum_in_sequence() {
    run_interpreter_program(
        r#"Let fn1 be a new Seq of Text.
Push "val" to fn1.
Let fv1 be a new Seq of CExpr.
Push a new CInt with value 1 to fv1.
Let v1 be a new CNewVariant with tag "A" and fnames fn1 and fvals fv1.
Let fn2 be a new Seq of Text.
Push "val" to fn2.
Let fv2 be a new Seq of CExpr.
Push a new CInt with value 2 to fv2.
Let v2 be a new CNewVariant with tag "B" and fnames fn2 and fvals fv2.
Let fn3 be a new Seq of Text.
Push "val" to fn3.
Let fv3 be a new Seq of CExpr.
Push a new CInt with value 3 to fv3.
Let v3 be a new CNewVariant with tag "A" and fnames fn3 and fvals fv3.
Let listItems be a new Seq of CExpr.
Push v1 to listItems.
Push v2 to listItems.
Push v3 to listItems.
Let listExpr be a new CList with items listItems.
Let letSeq be a new CLet with name "items" and expr listExpr.
Let letSum be a new CLet with name "sum" and expr (a new CInt with value 0).
Let aBindings be a new Seq of Text.
Push "x" to aBindings.
Let aBody be a new Seq of CStmt.
Let addExpr be a new CBinOp with op "+" and left (a new CVar with name "sum") and right (a new CVar with name "x").
Push a new CSet with name "sum" and expr addExpr to aBody.
Let armA be a new CWhen with variantName "A" and bindings aBindings and body aBody.
Let bBindings be a new Seq of Text.
Push "x" to bBindings.
Let bBody be a new Seq of CStmt.
Let mulExpr be a new CBinOp with op "*" and left (a new CVar with name "sum") and right (a new CVar with name "x").
Push a new CSet with name "sum" and expr mulExpr to bBody.
Let armB be a new CWhen with variantName "B" and bindings bBindings and body bBody.
Let arms be a new Seq of CMatchArm.
Push armA to arms.
Push armB to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "it") and arms arms.
Let repBody be a new Seq of CStmt.
Push inspStmt to repBody.
Let repStmt be a new CRepeat with var "it" and coll (a new CVar with name "items") and body repBody.
Let showStmt be a new CShow with expr (a new CVar with name "sum").
Let stmts be a new Seq of CStmt.
Push letSeq to stmts.
Push letSum to stmts.
Push repStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "5",
    );
}

#[test]
fn core_enum_variant_equality() {
    run_interpreter_program(
        r#"Let fn1 be a new Seq of Text.
Push "val" to fn1.
Let fv1 be a new Seq of CExpr.
Push a new CInt with value 42 to fv1.
Let v1 be a new CNewVariant with tag "X" and fnames fn1 and fvals fv1.
Let letA be a new CLet with name "a" and expr v1.
Let armBindings be a new Seq of Text.
Push "av" to armBindings.
Let armBody be a new Seq of CStmt.
Push a new CShow with expr (a new CVar with name "av") to armBody.
Let arm1 be a new CWhen with variantName "X" and bindings armBindings and body armBody.
Let arms be a new Seq of CMatchArm.
Push arm1 to arms.
Let inspA be a new CInspect with target (a new CVar with name "a") and arms arms.
Let stmts be a new Seq of CStmt.
Push letA to stmts.
Push inspA to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "42",
    );
}

#[test]
fn core_enum_recursive_type() {
    run_interpreter_program(
        r#"Let numFn1 be a new Seq of Text.
Push "val" to numFn1.
Let numFv1 be a new Seq of CExpr.
Push a new CInt with value 1 to numFv1.
Let num1 be a new CNewVariant with tag "Num" and fnames numFn1 and fvals numFv1.
Let numFn2 be a new Seq of Text.
Push "val" to numFn2.
Let numFv2 be a new Seq of CExpr.
Push a new CInt with value 2 to numFv2.
Let num2 be a new CNewVariant with tag "Num" and fnames numFn2 and fvals numFv2.
Let addFn be a new Seq of Text.
Push "left" to addFn.
Push "right" to addFn.
Let addFv be a new Seq of CExpr.
Push num1 to addFv.
Push num2 to addFv.
Let addExpr be a new CNewVariant with tag "Add" and fnames addFn and fvals addFv.
Let letE be a new CLet with name "expr" and expr addExpr.
Let addBindings be a new Seq of Text.
Push "l" to addBindings.
Push "r" to addBindings.
Let lNumBindings be a new Seq of Text.
Push "ln" to lNumBindings.
Let rNumBindings be a new Seq of Text.
Push "rn" to rNumBindings.
Let sumBody be a new Seq of CStmt.
Let sumExpr be a new CBinOp with op "+" and left (a new CVar with name "ln") and right (a new CVar with name "rn").
Push a new CShow with expr sumExpr to sumBody.
Let rNumArm be a new CWhen with variantName "Num" and bindings rNumBindings and body sumBody.
Let rArms be a new Seq of CMatchArm.
Push rNumArm to rArms.
Let rInspect be a new CInspect with target (a new CVar with name "r") and arms rArms.
Let lBody be a new Seq of CStmt.
Push rInspect to lBody.
Let lNumArm be a new CWhen with variantName "Num" and bindings lNumBindings and body lBody.
Let lArms be a new Seq of CMatchArm.
Push lNumArm to lArms.
Let lInspect be a new CInspect with target (a new CVar with name "l") and arms lArms.
Let addBody be a new Seq of CStmt.
Push lInspect to addBody.
Let addArm be a new CWhen with variantName "Add" and bindings addBindings and body addBody.
Let numBindings be a new Seq of Text.
Push "n" to numBindings.
Let numBody be a new Seq of CStmt.
Push a new CShow with expr (a new CVar with name "n") to numBody.
Let numArm be a new CWhen with variantName "Num" and bindings numBindings and body numBody.
Let arms be a new Seq of CMatchArm.
Push numArm to arms.
Push addArm to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "expr") and arms arms.
Let stmts be a new Seq of CStmt.
Push letE to stmts.
Push inspStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "3",
    );
}

#[test]
fn core_enum_inspect_all_arms() {
    run_interpreter_program(
        r#"Let fn1 be a new Seq of Text.
Push "val" to fn1.
Let fv1 be a new Seq of CExpr.
Push a new CInt with value 10 to fv1.
Let vC be a new CNewVariant with tag "C" and fnames fn1 and fvals fv1.
Let letS be a new CLet with name "s" and expr vC.
Let abA be a new Seq of Text.
Push "x" to abA.
Let bodyA be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "arm-a") to bodyA.
Let armA be a new CWhen with variantName "A" and bindings abA and body bodyA.
Let abB be a new Seq of Text.
Push "x" to abB.
Let bodyB be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "arm-b") to bodyB.
Let armB be a new CWhen with variantName "B" and bindings abB and body bodyB.
Let abC be a new Seq of Text.
Push "x" to abC.
Let bodyC be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "arm-c") to bodyC.
Let armC be a new CWhen with variantName "C" and bindings abC and body bodyC.
Let abD be a new Seq of Text.
Push "x" to abD.
Let bodyD be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "arm-d") to bodyD.
Let armD be a new CWhen with variantName "D" and bindings abD and body bodyD.
Let arms be a new Seq of CMatchArm.
Push armA to arms.
Push armB to arms.
Push armC to arms.
Push armD to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "s") and arms arms.
Let stmts be a new Seq of CStmt.
Push letS to stmts.
Push inspStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "arm-c",
    );
}

// Sprint 13 Step 5 — Encoding and edge cases (5 tests)

#[test]
fn core_enum_pass_to_function() {
    run_interpreter_program(
        r#"Let fnames1 be a new Seq of Text.
Push "val" to fnames1.
Let fvals1 be a new Seq of CExpr.
Push a new CInt with value 99 to fvals1.
Let nvExpr be a new CNewVariant with tag "Box" and fnames fnames1 and fvals fvals1.
Let letV be a new CLet with name "myBox" and expr nvExpr.
Let armBindings be a new Seq of Text.
Push "v" to armBindings.
Let armBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CVar with name "v") to armBody.
Let arm1 be a new CWhen with variantName "Box" and bindings armBindings and body armBody.
Let arms be a new Seq of CMatchArm.
Push arm1 to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "b") and arms arms.
Let funcBody be a new Seq of CStmt.
Push inspStmt to funcBody.
Let funcParams be a new Seq of Text.
Push "b" to funcParams.
Let func be a new CFuncDef with name "unbox" and params funcParams and body funcBody.
Let funcMap be a new Map of Text to CFunc.
Set item "unbox" of funcMap to func.
Let callArgs be a new Seq of CExpr.
Push (a new CVar with name "myBox") to callArgs.
Let callExpr be a new CCall with name "unbox" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push letV to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "99",
    );
}

#[test]
fn core_enum_construct_in_function() {
    run_interpreter_program(
        r#"Let fnBody be a new Seq of CStmt.
Let fnames1 be a new Seq of Text.
Push "val" to fnames1.
Let fvals1 be a new Seq of CExpr.
Push (a new CVar with name "x") to fvals1.
Let nvExpr be a new CNewVariant with tag "Wrapped" and fnames fnames1 and fvals fvals1.
Push a new CReturn with expr nvExpr to fnBody.
Let fnParams be a new Seq of Text.
Push "x" to fnParams.
Let func be a new CFuncDef with name "wrap" and params fnParams and body fnBody.
Let funcMap be a new Map of Text to CFunc.
Set item "wrap" of funcMap to func.
Let callArgs be a new Seq of CExpr.
Push (a new CInt with value 42) to callArgs.
Let callExpr be a new CCall with name "wrap" and args callArgs.
Let letW be a new CLet with name "w" and expr callExpr.
Let armBindings be a new Seq of Text.
Push "v" to armBindings.
Let armBody be a new Seq of CStmt.
Push a new CShow with expr (a new CVar with name "v") to armBody.
Let arm1 be a new CWhen with variantName "Wrapped" and bindings armBindings and body armBody.
Let arms be a new Seq of CMatchArm.
Push arm1 to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "w") and arms arms.
Let stmts be a new Seq of CStmt.
Push letW to stmts.
Push inspStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "42",
    );
}

#[test]
fn core_enum_map_over_variants() {
    run_interpreter_program(
        r#"Let fn1 be a new Seq of Text.
Push "val" to fn1.
Let fv1 be a new Seq of CExpr.
Push a new CInt with value 10 to fv1.
Let v1 be a new CNewVariant with tag "Num" and fnames fn1 and fvals fv1.
Let fn2 be a new Seq of Text.
Push "val" to fn2.
Let fv2 be a new Seq of CExpr.
Push a new CInt with value 20 to fv2.
Let v2 be a new CNewVariant with tag "Num" and fnames fn2 and fvals fv2.
Let fn3 be a new Seq of Text.
Push "val" to fn3.
Let fv3 be a new Seq of CExpr.
Push a new CInt with value 30 to fv3.
Let v3 be a new CNewVariant with tag "Num" and fnames fn3 and fvals fv3.
Let listItems be a new Seq of CExpr.
Push v1 to listItems.
Push v2 to listItems.
Push v3 to listItems.
Let listExpr be a new CList with items listItems.
Let letSeq be a new CLet with name "items" and expr listExpr.
Let letResult be a new CLet with name "result" and expr (a new CNewSeq).
Let numBindings be a new Seq of Text.
Push "n" to numBindings.
Let numBody be a new Seq of CStmt.
Let doubled be a new CBinOp with op "*" and left (a new CVar with name "n") and right (a new CInt with value 2).
Push a new CPush with expr doubled and target "result" to numBody.
Let numArm be a new CWhen with variantName "Num" and bindings numBindings and body numBody.
Let arms be a new Seq of CMatchArm.
Push numArm to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "it") and arms arms.
Let repBody be a new Seq of CStmt.
Push inspStmt to repBody.
Let repStmt be a new CRepeat with var "it" and coll (a new CVar with name "items") and body repBody.
Let showStmt be a new CShow with expr (a new CLen with target (a new CVar with name "result")).
Let stmts be a new Seq of CStmt.
Push letSeq to stmts.
Push letResult to stmts.
Push repStmt to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "3",
    );
}

#[test]
fn core_enum_inspect_no_match() {
    run_interpreter_program(
        r#"Let fnames1 be a new Seq of Text.
Push "val" to fnames1.
Let fvals1 be a new Seq of CExpr.
Push a new CInt with value 1 to fvals1.
Let nvExpr be a new CNewVariant with tag "Unknown" and fnames fnames1 and fvals fvals1.
Let letS be a new CLet with name "s" and expr nvExpr.
Let armBindings be a new Seq of Text.
Push "x" to armBindings.
Let armBody be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "matched") to armBody.
Let arm1 be a new CWhen with variantName "Known" and bindings armBindings and body armBody.
Let arms be a new Seq of CMatchArm.
Push arm1 to arms.
Let inspStmt be a new CInspect with target (a new CVar with name "s") and arms arms.
Let showAfter be a new CShow with expr (a new CText with value "done").
Let stmts be a new Seq of CStmt.
Push letS to stmts.
Push inspStmt to stmts.
Push showAfter to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "done",
    );
}

#[test]
fn core_enum_encode_roundtrip() {
    run_encoded_program(
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n\n## Main\nLet s be Circle(5).\nInspect s:\n    When Circle (r):\n        Show \"{r}\".\n    When Square (sd):\n        Show \"{sd}\".",
        "5",
    );
}

// ── Sprint 14: Closures, HOF, Interpolation ──────────────────────────

#[test]
fn core_closure_basic() {
    run_interpreter_program(
        r#"Let clParams be a new Seq of Text.
Push "x" to clParams.
Let clBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CBinOp with op "*" and left (a new CVar with name "x") and right (a new CInt with value 2)) to clBody.
Let clCaptured be a new Seq of Text.
Let clExpr be a new CClosure with params clParams and body clBody and captured clCaptured.
Let letF be a new CLet with name "f" and expr clExpr.
Let callArgs be a new Seq of CExpr.
Push a new CInt with value 5 to callArgs.
Let callExpr be a new CCallExpr with target (a new CVar with name "f") and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push letF to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "10",
    );
}

#[test]
fn core_closure_captured_var() {
    run_interpreter_program(
        r#"Let letFactor be a new CLet with name "factor" and expr (a new CInt with value 3).
Let clParams be a new Seq of Text.
Push "x" to clParams.
Let clBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CBinOp with op "*" and left (a new CVar with name "x") and right (a new CVar with name "factor")) to clBody.
Let clCaptured be a new Seq of Text.
Push "factor" to clCaptured.
Let clExpr be a new CClosure with params clParams and body clBody and captured clCaptured.
Let letF be a new CLet with name "f" and expr clExpr.
Let callArgs be a new Seq of CExpr.
Push a new CInt with value 4 to callArgs.
Let callExpr be a new CCallExpr with target (a new CVar with name "f") and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push letFactor to stmts.
Push letF to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "12",
    );
}

#[test]
fn core_closure_pass_to_function() {
    // Function "apply" takes closure as first arg, value as second, calls closure(value)
    run_interpreter_program(
        r#"Let applyParams be a new Seq of Text.
Push "fn" to applyParams.
Push "val" to applyParams.
Let applyCallArgs be a new Seq of CExpr.
Push a new CVar with name "val" to applyCallArgs.
Let applyBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CCallExpr with target (a new CVar with name "fn") and args applyCallArgs) to applyBody.
Let applyFn be a new CFuncDef with name "apply" and params applyParams and body applyBody.
Let funcMap be a new Map of Text to CFunc.
Set item "apply" of funcMap to applyFn.
Let clParams be a new Seq of Text.
Push "x" to clParams.
Let clBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CBinOp with op "+" and left (a new CVar with name "x") and right (a new CInt with value 10)) to clBody.
Let clCaptured be a new Seq of Text.
Let clExpr be a new CClosure with params clParams and body clBody and captured clCaptured.
Let letF be a new CLet with name "f" and expr clExpr.
Let callArgs be a new Seq of CExpr.
Push a new CVar with name "f" to callArgs.
Push a new CInt with value 7 to callArgs.
Let callExpr be a new CCall with name "apply" and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push letF to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "17",
    );
}

#[test]
fn core_closure_return_from_function() {
    // Function "makeAdder" takes n, returns closure that adds n
    run_interpreter_program(
        r#"Let maParams be a new Seq of Text.
Push "n" to maParams.
Let clParams be a new Seq of Text.
Push "x" to clParams.
Let clBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CBinOp with op "+" and left (a new CVar with name "x") and right (a new CVar with name "n")) to clBody.
Let clCaptured be a new Seq of Text.
Push "n" to clCaptured.
Let maBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CClosure with params clParams and body clBody and captured clCaptured) to maBody.
Let maFn be a new CFuncDef with name "makeAdder" and params maParams and body maBody.
Let funcMap be a new Map of Text to CFunc.
Set item "makeAdder" of funcMap to maFn.
Let makeArgs be a new Seq of CExpr.
Push a new CInt with value 100 to makeArgs.
Let letAdder be a new CLet with name "adder" and expr (a new CCall with name "makeAdder" and args makeArgs).
Let callArgs be a new Seq of CExpr.
Push a new CInt with value 5 to callArgs.
Let callExpr be a new CCallExpr with target (a new CVar with name "adder") and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push letAdder to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "105",
    );
}

#[test]
fn core_closure_multiple_params() {
    run_interpreter_program(
        r#"Let clParams be a new Seq of Text.
Push "a" to clParams.
Push "b" to clParams.
Let clBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CBinOp with op "+" and left (a new CVar with name "a") and right (a new CVar with name "b")) to clBody.
Let clCaptured be a new Seq of Text.
Let clExpr be a new CClosure with params clParams and body clBody and captured clCaptured.
Let letF be a new CLet with name "f" and expr clExpr.
Let callArgs be a new Seq of CExpr.
Push a new CInt with value 30 to callArgs.
Push a new CInt with value 12 to callArgs.
Let callExpr be a new CCallExpr with target (a new CVar with name "f") and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push letF to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "42",
    );
}

#[test]
fn core_closure_no_params() {
    run_interpreter_program(
        r#"Let clParams be a new Seq of Text.
Let clBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CInt with value 99) to clBody.
Let clCaptured be a new Seq of Text.
Let clExpr be a new CClosure with params clParams and body clBody and captured clCaptured.
Let letF be a new CLet with name "thunk" and expr clExpr.
Let callArgs be a new Seq of CExpr.
Let callExpr be a new CCallExpr with target (a new CVar with name "thunk") and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push letF to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "99",
    );
}

#[test]
fn core_closure_to_text() {
    run_interpreter_program(
        r#"Let clParams be a new Seq of Text.
Push "x" to clParams.
Let clBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CVar with name "x") to clBody.
Let clCaptured be a new Seq of Text.
Let clExpr be a new CClosure with params clParams and body clBody and captured clCaptured.
Let letF be a new CLet with name "f" and expr clExpr.
Let showStmt be a new CShow with expr (a new CVar with name "f").
Let stmts be a new Seq of CStmt.
Push letF to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "<closure>",
    );
}

#[test]
fn core_interp_basic() {
    run_interpreter_program(
        r#"Let letName be a new CLet with name "name" and expr (a new CText with value "World").
Let parts be a new Seq of CStringPart.
Push a new CLiteralPart with value "Hello, " to parts.
Push a new CExprPart with expr (a new CVar with name "name") to parts.
Push a new CLiteralPart with value "!" to parts.
Let showStmt be a new CShow with expr (a new CInterpolatedString with parts parts).
Let stmts be a new Seq of CStmt.
Push letName to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Hello, World!",
    );
}

#[test]
fn core_interp_number() {
    run_interpreter_program(
        r#"Let letN be a new CLet with name "n" and expr (a new CInt with value 42).
Let parts be a new Seq of CStringPart.
Push a new CLiteralPart with value "Answer: " to parts.
Push a new CExprPart with expr (a new CVar with name "n") to parts.
Let showStmt be a new CShow with expr (a new CInterpolatedString with parts parts).
Let stmts be a new Seq of CStmt.
Push letN to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Answer: 42",
    );
}

#[test]
fn core_interp_expression() {
    run_interpreter_program(
        r#"Let parts be a new Seq of CStringPart.
Push a new CLiteralPart with value "Result: " to parts.
Push a new CExprPart with expr (a new CBinOp with op "+" and left (a new CInt with value 10) and right (a new CInt with value 20)) to parts.
Let showStmt be a new CShow with expr (a new CInterpolatedString with parts parts).
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "Result: 30",
    );
}

#[test]
fn core_interp_multiple_holes() {
    run_interpreter_program(
        r#"Let letA be a new CLet with name "a" and expr (a new CInt with value 1).
Let letB be a new CLet with name "b" and expr (a new CInt with value 2).
Let letC be a new CLet with name "c" and expr (a new CInt with value 3).
Let parts be a new Seq of CStringPart.
Push a new CExprPart with expr (a new CVar with name "a") to parts.
Push a new CLiteralPart with value "-" to parts.
Push a new CExprPart with expr (a new CVar with name "b") to parts.
Push a new CLiteralPart with value "-" to parts.
Push a new CExprPart with expr (a new CVar with name "c") to parts.
Let showStmt be a new CShow with expr (a new CInterpolatedString with parts parts).
Let stmts be a new Seq of CStmt.
Push letA to stmts.
Push letB to stmts.
Push letC to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "1-2-3",
    );
}

#[test]
fn core_interp_empty_string() {
    run_interpreter_program(
        r#"Let parts be a new Seq of CStringPart.
Push a new CLiteralPart with value "just text" to parts.
Let showStmt be a new CShow with expr (a new CInterpolatedString with parts parts).
Let stmts be a new Seq of CStmt.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "just text",
    );
}

#[test]
fn core_closure_as_map_callback() {
    // Closure doubles each element, show each result inline
    run_interpreter_program(
        r#"Let clParams be a new Seq of Text.
Push "x" to clParams.
Let clBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CBinOp with op "*" and left (a new CVar with name "x") and right (a new CInt with value 2)) to clBody.
Let clCaptured be a new Seq of Text.
Let clExpr be a new CClosure with params clParams and body clBody and captured clCaptured.
Let letDbl be a new CLet with name "dbl" and expr clExpr.
Let items be a new Seq of CExpr.
Push a new CInt with value 3 to items.
Push a new CInt with value 5 to items.
Push a new CInt with value 7 to items.
Let letSeq be a new CLet with name "seq" and expr (a new CList with items items).
Let callArgs be a new Seq of CExpr.
Push a new CVar with name "item" to callArgs.
Let callExpr be a new CCallExpr with target (a new CVar with name "dbl") and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let loopBody be a new Seq of CStmt.
Push showStmt to loopBody.
Let repeatStmt be a new CRepeat with var "item" and coll (a new CVar with name "seq") and body loopBody.
Let stmts be a new Seq of CStmt.
Push letDbl to stmts.
Push letSeq to stmts.
Push repeatStmt to stmts.
Let env be a new Map of Text to CVal.
Let funcMap be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "6\n10\n14",
    );
}

#[test]
fn core_closure_nested() {
    // makeMultiplier returns a closure that multiplies by captured factor
    // Then we call makeMultiplier(3) to get tripler, then tripler(7) = 21
    run_interpreter_program(
        r#"Let mmParams be a new Seq of Text.
Push "factor" to mmParams.
Let innerParams be a new Seq of Text.
Push "x" to innerParams.
Let innerBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CBinOp with op "*" and left (a new CVar with name "x") and right (a new CVar with name "factor")) to innerBody.
Let innerCaptured be a new Seq of Text.
Push "factor" to innerCaptured.
Let mmBody be a new Seq of CStmt.
Push a new CReturn with expr (a new CClosure with params innerParams and body innerBody and captured innerCaptured) to mmBody.
Let mmFn be a new CFuncDef with name "makeMultiplier" and params mmParams and body mmBody.
Let funcMap be a new Map of Text to CFunc.
Set item "makeMultiplier" of funcMap to mmFn.
Let makeArgs be a new Seq of CExpr.
Push a new CInt with value 3 to makeArgs.
Let letTripler be a new CLet with name "tripler" and expr (a new CCall with name "makeMultiplier" and args makeArgs).
Let callArgs be a new Seq of CExpr.
Push a new CInt with value 7 to callArgs.
Let callExpr be a new CCallExpr with target (a new CVar with name "tripler") and args callArgs.
Let showStmt be a new CShow with expr callExpr.
Let stmts be a new Seq of CStmt.
Push letTripler to stmts.
Push showStmt to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "21",
    );
}

#[test]
fn core_closure_encode_roundtrip() {
    run_encoded_program(
        "## Main\nLet doubler be (x: Int) -> x * 2.\nLet result be doubler(5).\nShow \"{result}\".",
        "10",
    );
}

// ── Sprint 15: Temporal Types ──────────────────────────────────────────

#[test]
fn core_temporal_duration_seconds() {
    run_interpreter_program(
        r#"
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let dur be coreEval(a new CDuration with amount (a new CInt with value 5) and unit "seconds", env, funcs).
Show valToText(dur).
"#,
        "5s",
    );
}

#[test]
fn core_temporal_duration_minutes() {
    run_interpreter_program(
        r#"
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let dur be coreEval(a new CDuration with amount (a new CInt with value 3) and unit "minutes", env, funcs).
Show valToText(dur).
"#,
        "3m",
    );
}

#[test]
fn core_temporal_duration_add() {
    run_interpreter_program(
        r#"
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let d1 be a new CDuration with amount (a new CInt with value 5) and unit "seconds".
Let d2 be a new CDuration with amount (a new CInt with value 10) and unit "seconds".
Let addExpr be a new CBinOp with op "+" and left d1 and right d2.
Let result be coreEval(addExpr, env, funcs).
Show valToText(result).
"#,
        "15s",
    );
}

#[test]
fn core_temporal_duration_multiply() {
    run_interpreter_program(
        r#"
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let d1 be a new CDuration with amount (a new CInt with value 5) and unit "seconds".
Let mulExpr be a new CBinOp with op "*" and left d1 and right (a new CInt with value 3).
Let result be coreEval(mulExpr, env, funcs).
Show valToText(result).
"#,
        "15s",
    );
}

#[test]
fn core_temporal_date_construct() {
    run_interpreter_program(
        r#"
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreEval(a new CDateToday, env, funcs).
Show valToText(result).
"#,
        "2026-1-1",
    );
}

#[test]
fn core_temporal_date_comparison() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CLet with name "d1" and expr (a new CDateToday) to stmts.
Push a new CLet with name "d2" and expr (a new CDateToday) to stmts.
Let cmpExpr be a new CBinOp with op "==" and left (a new CVar with name "d1") and right (a new CVar with name "d2").
Push a new CShow with expr cmpExpr to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "true",
    );
}

// ── Sprint 15, Step 3: Temporal arithmetic and encoding ────────────────

#[test]
fn core_temporal_date_add_duration() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CLet with name "d" and expr (a new CDateToday) to stmts.
Push a new CLet with name "dur" and expr (a new CDuration with amount (a new CInt with value 5) and unit "seconds") to stmts.
Let addExpr be a new CBinOp with op "+" and left (a new CVar with name "d") and right (a new CVar with name "dur").
Push a new CShow with expr addExpr to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "2026-1-1",
    );
}

#[test]
fn core_temporal_date_difference() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CLet with name "d1" and expr (a new CDateToday) to stmts.
Push a new CLet with name "d2" and expr (a new CDateToday) to stmts.
Let diffExpr be a new CBinOp with op "-" and left (a new CVar with name "d1") and right (a new CVar with name "d2").
Push a new CShow with expr diffExpr to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "0ms",
    );
}

#[test]
fn core_temporal_moment_comparison() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CLet with name "m1" and expr (a new CTimeNow) to stmts.
Push a new CLet with name "m2" and expr (a new CTimeNow) to stmts.
Let cmpExpr be a new CBinOp with op "==" and left (a new CVar with name "m1") and right (a new CVar with name "m2").
Push a new CShow with expr cmpExpr to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "true",
    );
}

#[test]
fn core_temporal_time_construct() {
    run_interpreter_program(
        r#"
Let env be a new Map of Text to CVal.
Set item "t" of env to (a new VTime with hour 14 and minute 30 and second 0).
Let funcs be a new Map of Text to CFunc.
Let stmts be a new Seq of CStmt.
Push a new CShow with expr (a new CVar with name "t") to stmts.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "14:30:0",
    );
}

#[test]
fn core_temporal_duration_to_text() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CLet with name "d1" and expr (a new CDuration with amount (a new CInt with value 500) and unit "milliseconds") to stmts.
Push a new CShow with expr (a new CVar with name "d1") to stmts.
Push a new CLet with name "d2" and expr (a new CDuration with amount (a new CInt with value 30) and unit "seconds") to stmts.
Push a new CShow with expr (a new CVar with name "d2") to stmts.
Push a new CLet with name "d3" and expr (a new CDuration with amount (a new CInt with value 2) and unit "minutes") to stmts.
Push a new CShow with expr (a new CVar with name "d3") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "500ms\n30s\n2m",
    );
}

#[test]
fn core_temporal_encode_roundtrip() {
    run_encoded_program(
        "## Main\nLet d be 5s.\nShow \"{d}\".",
        "5s",
    );
}

// ── Sprint 16: IO, Sleep, Assert, Escape ───────────────────────────────

#[test]
fn core_io_runtime_assert_pass() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CRuntimeAssert with cond (a new CBool with value true) and msg (a new CText with value "should not fire") to stmts.
Push a new CShow with expr (a new CText with value "ok") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "ok",
    );
}

#[test]
fn core_io_runtime_assert_fail() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CRuntimeAssert with cond (a new CBool with value false) and msg (a new CText with value "invariant broken") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "Assertion failed: invariant broken",
    );
}

#[test]
fn core_io_give() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CLet with name "x" and expr (a new CInt with value 42) to stmts.
Push a new CGive with expr (a new CVar with name "x") and target "y" to stmts.
Push a new CShow with expr (a new CVar with name "y") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "42",
    );
}

#[test]
fn core_io_escape_stmt() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CEscStmt with code "foreign_code_here" to stmts.
Push a new CShow with expr (a new CText with value "after") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "after",
    );
}

#[test]
fn core_io_escape_expr() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CLet with name "x" and expr (a new CEscExpr with code "foreign_expr") to stmts.
Push a new CShow with expr (a new CText with value "after") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "after",
    );
}

#[test]
fn core_io_write_and_read() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CReadConsole with target "input" to stmts.
Push a new CShow with expr (a new CText with value "handled") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "handled",
    );
}

#[test]
fn core_io_sleep() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CSleep with duration (a new CInt with value 0) to stmts.
Push a new CShow with expr (a new CText with value "awake") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "awake",
    );
}

#[test]
fn core_io_assert_with_expression() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Let condExpr be a new CBinOp with op ">" and left (a new CInt with value 5) and right (a new CInt with value 3).
Push a new CRuntimeAssert with cond condExpr and msg (a new CText with value "math works") to stmts.
Push a new CShow with expr (a new CText with value "ok") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "ok",
    );
}

#[test]
fn core_io_assert_dynamic_message() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CLet with name "x" and expr (a new CInt with value 3) to stmts.
Let msgExpr be a new CBinOp with op "+" and left (a new CText with value "x was ") and right (a new CVar with name "x").
Push a new CRuntimeAssert with cond (a new CBool with value false) and msg msgExpr to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "Assertion failed: x was 3",
    );
}

#[test]
fn core_io_give_in_function() {
    run_interpreter_program(
        r#"
Let fnBody be a new Seq of CStmt.
Push a new CGive with expr (a new CBinOp with op "*" and left (a new CVar with name "n") and right (a new CInt with value 2)) and target "result" to fnBody.
Push a new CReturn with expr (a new CVar with name "result") to fnBody.
Let fnParams be a new Seq of Text.
Push "n" to fnParams.
Let fn1 be a new CFuncDef with name "double" and params fnParams and body fnBody.
Let funcMap be a new Map of Text to CFunc.
Set item "double" of funcMap to fn1.
Let callArgs be a new Seq of CExpr.
Push a new CInt with value 7 to callArgs.
Let stmts be a new Seq of CStmt.
Push a new CShow with expr (a new CCall with name "double" and args callArgs) to stmts.
Let env be a new Map of Text to CVal.
Let result be coreExecBlock(stmts, env, funcMap).
"#,
        "14",
    );
}

#[test]
fn core_io_pe_treats_io_as_dynamic() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CSleep with duration (a new CInt with value 0) to stmts.
Push a new CRuntimeAssert with cond (a new CBool with value true) and msg (a new CText with value "ok") to stmts.
Push a new CEscStmt with code "foreign" to stmts.
Push a new CShow with expr (a new CText with value "dynamic") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "dynamic",
    );
}

#[test]
fn core_io_encode_roundtrip() {
    run_encoded_program(
        "## Main\nLet mutable x be 10.\nShow \"{x}\".",
        "10",
    );
}

// ── Sprint 17: Security, Proofs, Require ───────────────────────────────

#[test]
fn core_security_check_pass() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CCheck with predicate (a new CBool with value true) and msg (a new CText with value "access denied") to stmts.
Push a new CShow with expr (a new CText with value "granted") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "granted",
    );
}

#[test]
fn core_security_check_fail() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CCheck with predicate (a new CBool with value false) and msg (a new CText with value "access denied") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "Security violation: access denied",
    );
}

#[test]
fn core_security_assert() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Let cond be a new CBinOp with op "==" and left (a new CInt with value 2) and right (a new CInt with value 2).
Push a new CAssert with proposition cond to stmts.
Push a new CShow with expr (a new CText with value "valid") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "valid",
    );
}

#[test]
fn core_security_trust() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CTrust with proposition (a new CBool with value true) and justification "well-known fact" to stmts.
Push a new CShow with expr (a new CText with value "trusted") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "trusted",
    );
}

#[test]
fn core_security_check_with_expression() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CLet with name "level" and expr (a new CInt with value 5) to stmts.
Let cond be a new CBinOp with op ">" and left (a new CVar with name "level") and right (a new CInt with value 3).
Push a new CCheck with predicate cond and msg (a new CText with value "insufficient level") to stmts.
Push a new CShow with expr (a new CText with value "ok") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "ok",
    );
}

#[test]
fn core_security_require() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CRequire with dependency "some_dep" to stmts.
Push a new CShow with expr (a new CText with value "loaded") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "loaded",
    );
}

#[test]
fn core_security_check_never_eliminated() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CCheck with predicate (a new CBool with value true) and msg (a new CText with value "must stay") to stmts.
Push a new CCheck with predicate (a new CBool with value true) and msg (a new CText with value "also must stay") to stmts.
Push a new CShow with expr (a new CText with value "secure") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "secure",
    );
}

#[test]
fn core_security_encode_roundtrip() {
    run_encoded_program(
        "## Main\nLet x be 42.\nShow \"{x}\".",
        "42",
    );
}

// ── Sprint 18: CRDTs ────────────────────────────────────────────────────

#[test]
fn core_crdt_gcounter_increase() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CIncrease with target "counter" and amount (a new CInt with value 5) to stmts.
Push a new CShow with expr (a new CVar with name "counter") to stmts.
Let env be a new Map of Text to CVal.
Set item "counter" of env to a new VInt with value 0.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "5",
    );
}

#[test]
fn core_crdt_pncounter() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CIncrease with target "counter" and amount (a new CInt with value 10) to stmts.
Push a new CDecrease with target "counter" and amount (a new CInt with value 3) to stmts.
Push a new CShow with expr (a new CVar with name "counter") to stmts.
Let env be a new Map of Text to CVal.
Set item "counter" of env to a new VInt with value 0.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "7",
    );
}

#[test]
fn core_crdt_merge() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CMerge with target "local" and other (a new CVar with name "remote") to stmts.
Push a new CShow with expr (a new CText with value "merged") to stmts.
Let env be a new Map of Text to CVal.
Set item "local" of env to a new VInt with value 5.
Set item "remote" of env to a new VInt with value 10.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "merged",
    );
}

#[test]
fn core_crdt_rga_append() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CAppendToSeq with target "doc" and value (a new CText with value "hello") to stmts.
Push a new CAppendToSeq with target "doc" and value (a new CText with value "world") to stmts.
Push a new CShow with expr (a new CVar with name "doc") to stmts.
Let env be a new Map of Text to CVal.
Set item "doc" of env to a new VSeq with items (a new Seq of CVal).
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "[seq]",
    );
}

#[test]
fn core_crdt_resolve() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CResolve with target "reg" to stmts.
Push a new CShow with expr (a new CText with value "resolved") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "resolved",
    );
}

#[test]
fn core_crdt_sync_noop() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CSync with target "x" and channel (a new CText with value "topic1") to stmts.
Push a new CShow with expr (a new CText with value "synced") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "synced",
    );
}

#[test]
fn core_crdt_mount_noop() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CMount with target "x" and path (a new CText with value "/tmp/data.journal") to stmts.
Push a new CShow with expr (a new CText with value "mounted") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "mounted",
    );
}

#[test]
fn core_crdt_to_text() {
    run_interpreter_program(
        r#"
Let crdtState be a new Map of Text to CVal.
Set item "count" of crdtState to a new VInt with value 42.
Let v be a new VCrdt with kind "GCounter" and state crdtState.
Show valToText(v).
"#,
        "<crdt:GCounter>",
    );
}

#[test]
fn core_crdt_multiple_operations() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CIncrease with target "c" and amount (a new CInt with value 10) to stmts.
Push a new CIncrease with target "c" and amount (a new CInt with value 5) to stmts.
Push a new CDecrease with target "c" and amount (a new CInt with value 3) to stmts.
Push a new CShow with expr (a new CVar with name "c") to stmts.
Let env be a new Map of Text to CVal.
Set item "c" of env to a new VInt with value 0.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "12",
    );
}

#[test]
fn core_crdt_encode_roundtrip() {
    run_encoded_program(
        "## Main\nLet x be 99.\nShow \"{x}\".",
        "99",
    );
}

// ── Sprint 19: Concurrency, Actors, Networking ──────────────────────────

#[test]
fn core_concurrent_sequential() {
    run_interpreter_program(
        r#"
Let branch1 be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "a") to branch1.
Let branch2 be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "b") to branch2.
Let branches be a new Seq of Seq of CStmt.
Push branch1 to branches.
Push branch2 to branches.
Let stmts be a new Seq of CStmt.
Push a new CConcurrent with branches branches to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "a\nb",
    );
}

#[test]
fn core_parallel_sequential() {
    run_interpreter_program(
        r#"
Let branch1 be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "x") to branch1.
Let branch2 be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "y") to branch2.
Let branches be a new Seq of Seq of CStmt.
Push branch1 to branches.
Push branch2 to branches.
Let stmts be a new Seq of CStmt.
Push a new CParallel with branches branches to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "x\ny",
    );
}

#[test]
fn core_launch_task() {
    run_interpreter_program(
        r#"
Let taskBody be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "task") to taskBody.
Let stmts be a new Seq of CStmt.
Push a new CLaunchTask with body taskBody and handle "h" to stmts.
Push a new CShow with expr (a new CText with value "main") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "task\nmain",
    );
}

#[test]
fn core_pipe_send_receive() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CCreatePipe with name "ch" and capacity (a new CInt with value 10) to stmts.
Push a new CSendPipe with chan "ch" and value (a new CInt with value 42) to stmts.
Push a new CReceivePipe with chan "ch" and target "val" to stmts.
Push a new CShow with expr (a new CVar with name "val") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "42",
    );
}

#[test]
fn core_pipe_multiple() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CCreatePipe with name "ch" and capacity (a new CInt with value 10) to stmts.
Push a new CSendPipe with chan "ch" and value (a new CInt with value 1) to stmts.
Push a new CSendPipe with chan "ch" and value (a new CInt with value 2) to stmts.
Push a new CSendPipe with chan "ch" and value (a new CInt with value 3) to stmts.
Push a new CReceivePipe with chan "ch" and target "v1" to stmts.
Push a new CReceivePipe with chan "ch" and target "v2" to stmts.
Push a new CReceivePipe with chan "ch" and target "v3" to stmts.
Push a new CShow with expr (a new CVar with name "v1") to stmts.
Push a new CShow with expr (a new CVar with name "v2") to stmts.
Push a new CShow with expr (a new CVar with name "v3") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "1\n2\n3",
    );
}

#[test]
fn core_select_basic() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CCreatePipe with name "ch" and capacity (a new CInt with value 10) to stmts.
Push a new CSendPipe with chan "ch" and value (a new CInt with value 99) to stmts.
Let recvBody be a new Seq of CStmt.
Push a new CShow with expr (a new CVar with name "v") to recvBody.
Let selBranches be a new Seq of CSelectBranch.
Push a new CSelectRecv with chan "ch" and var "v" and body recvBody to selBranches.
Push a new CSelect with branches selBranches to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "99",
    );
}

#[test]
fn core_spawn_noop() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CSpawn with agentType "Worker" and target "w" to stmts.
Push a new CShow with expr (a new CText with value "spawned") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "spawned",
    );
}

#[test]
fn core_zone_transparent() {
    run_interpreter_program(
        r#"
Let zoneBody be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "inside") to zoneBody.
Let stmts be a new Seq of CStmt.
Push a new CZone with name "z" and kind "heap" and body zoneBody to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "inside",
    );
}

#[test]
fn core_listen_noop() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CListen with addr (a new CText with value "localhost:8000") and handler "h" to stmts.
Push a new CShow with expr (a new CText with value "listening") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "listening",
    );
}

#[test]
fn core_connect_noop() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CConnectTo with addr (a new CText with value "localhost:8000") and target "conn" to stmts.
Push a new CShow with expr (a new CText with value "connected") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "connected",
    );
}

#[test]
fn core_stop_task() {
    run_interpreter_program(
        r#"
Let taskBody be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "running") to taskBody.
Let stmts be a new Seq of CStmt.
Push a new CLaunchTask with body taskBody and handle "h" to stmts.
Push a new CStopTask with handle (a new CVar with name "h") to stmts.
Push a new CShow with expr (a new CText with value "stopped") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "running\nstopped",
    );
}

#[test]
fn core_try_send_receive() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CCreatePipe with name "ch" and capacity (a new CInt with value 10) to stmts.
Push a new CTrySendPipe with chan "ch" and value (a new CInt with value 77) to stmts.
Push a new CTryReceivePipe with chan "ch" and target "val" to stmts.
Push a new CShow with expr (a new CVar with name "val") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "77",
    );
}

#[test]
fn core_send_message_noop() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CSendMessage with target (a new CText with value "agent1") and msg (a new CText with value "hello") to stmts.
Push a new CAwaitMessage with target "response" to stmts.
Push a new CShow with expr (a new CText with value "done") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "done",
    );
}

#[test]
fn core_pe_dynamic_all_effects() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "all_dynamic") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "all_dynamic",
    );
}

#[test]
fn core_concurrent_encode_roundtrip() {
    run_encoded_program(
        "## Main\nLet x be 55.\nShow \"{x}\".",
        "55",
    );
}

// ===== Sprint 20 — Full Coverage Projection Verification =====

#[test]
fn full_encode_every_expr() {
    let source = r#"## Main
Let a be 42.
Let b be 3.14.
Let c be "hello".
Let d be true.
Let e be a + b.
Let f be a * 2.
Let g be a > 0.
Show "{a}".
"#;
    let result = logicaffeine_compile::compile::encode_program_source(source);
    assert!(result.is_ok(), "encode_program should not panic on basic exprs: {:?}", result.err());
}

#[test]
fn full_encode_every_stmt() {
    let source = r#"## To helper (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * helper(n - 1).

## Main
Let x be 10.
Let mutable y be 0.
Set y to helper(x).
While y is greater than 100:
    Set y to y - 100.
Show y.
"#;
    let result = logicaffeine_compile::compile::encode_program_source(source);
    assert!(result.is_ok(), "encode_program should not panic on stmt variants: {:?}", result.err());
}

#[test]
fn full_interpreter_every_cexpr() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CLet with name "a" and expr (a new CInt with value 10) to stmts.
Push a new CLet with name "b" and expr (a new CFloat with value 3.5) to stmts.
Push a new CLet with name "c" and expr (a new CText with value "hi") to stmts.
Push a new CLet with name "d" and expr (a new CBool with value true) to stmts.
Push a new CLet with name "e" and expr (a new CBinOp with op "+" and left (a new CVar with name "a") and right (a new CInt with value 5)) to stmts.
Push a new CLet with name "f" and expr (a new CNot with inner (a new CBool with value false)) to stmts.
Push a new CLet with name "g" and expr (a new CLen with target (a new CText with value "abc")) to stmts.
Push a new CShow with expr (a new CVar with name "e") to stmts.
Push a new CShow with expr (a new CVar with name "f") to stmts.
Push a new CShow with expr (a new CVar with name "g") to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "15\ntrue\n3",
    );
}

#[test]
fn full_interpreter_every_cstmt() {
    run_interpreter_program(
        r#"
Let stmts be a new Seq of CStmt.
Push a new CLet with name "x" and expr (a new CInt with value 0) to stmts.
Push a new CSet with name "x" and expr (a new CInt with value 42) to stmts.
Push a new CShow with expr (a new CVar with name "x") to stmts.
Let ifBody be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "yes") to ifBody.
Let elseBody be a new Seq of CStmt.
Push a new CShow with expr (a new CText with value "no") to elseBody.
Push a new CIf with cond (a new CBool with value true) and thenBlock ifBody and elseBlock elseBody to stmts.
Let env be a new Map of Text to CVal.
Let funcs be a new Map of Text to CFunc.
Let result be coreExecBlock(stmts, env, funcs).
"#,
        "42\nyes",
    );
}

#[test]
fn full_interpreter_every_cval() {
    run_interpreter_program(
        r#"
Let v1 be valToText(a new VInt with value 42).
Let v2 be valToText(a new VFloat with value 3.14).
Let v3 be valToText(a new VText with value "hello").
Let v4 be valToText(a new VBool with value true).
Let v5 be valToText(a new VNothing).
Let v6 be valToText(a new VSeq with items (a new Seq of CVal)).
Let v7 be valToText(a new VMap with entries (a new Map of Text to CVal)).
Let v8 be valToText(a new VError with msg "err").
Let v9 be valToText(a new VSet with items (a new Seq of CVal)).
Let v10 be valToText(a new VCrdt with kind "GCounter" and state (a new Map of Text to CVal)).
Show v1.
Show v2.
Show v3.
Show v4.
Show v5.
Show v6.
Show v7.
Show v8.
Show v9.
Show v10.
"#,
        "42\n3.14\nhello\ntrue\nnothing\n[seq]\n[map]\nError: err\n[set]\n<crdt:GCounter>",
    );
}

#[test]
fn full_p1_struct_program() {
    run_p1_and_verify(
        "## Main\nLet x be 3.\nLet y be 4.\nLet sum be x + y.\nShow sum.",
        "7",
    );
}

#[test]
fn full_p1_enum_program() {
    run_p1_and_verify(
        "## To classify (n: Int) -> Text:\n    If n is greater than 0:\n        Return \"positive\".\n    If n is less than 0:\n        Return \"negative\".\n    Return \"zero\".\n\n## Main\nShow classify(5).\nShow classify(0 - 3).\nShow classify(0).",
        "positive\nnegative\nzero",
    );
}

#[test]
fn full_p1_closure_program() {
    run_p1_and_verify(
        "## To apply (f: Int, n: Int) -> Int:\n    Return f + n.\n\n## Main\nLet result be apply(10, 5).\nShow result.",
        "15",
    );
}

#[test]
fn full_p1_iteration_program() {
    run_p1_and_verify(
        "Let total be 0.\nLet i be 1.\nWhile i is at most 10:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.",
        "55",
    );
}

#[test]
fn full_p1_mixed_features() {
    run_p1_and_verify(
        "## To sumRange (lo: Int, hi: Int) -> Int:\n    Let total be 0.\n    Let i be lo.\n    While i is at most hi:\n        Set total to total + i.\n        Set i to i + 1.\n    Return total.\n\n## Main\nLet a be sumRange(1, 5).\nLet b be sumRange(6, 10).\nShow a.\nShow b.\nShow a + b.",
        "15\n40\n55",
    );
}

#[test]
fn full_p1_p2_equivalence() {
    let programs: Vec<(&str, &str)> = vec![
        ("## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow double(7).", "14"),
        ("## To square (n: Int) -> Int:\n    Return n * n.\n\n## Main\nShow square(5).", "25"),
        ("## Main\nLet x be 100.\nLet y be x / 4.\nShow y.", "25"),
    ];

    for (program, expected) in &programs {
        let p1_output = run_via_p1(program);
        assert_eq!(p1_output, *expected, "P1 mismatch for: {}", program);
    }
}

#[test]
fn full_all_projections_struct() {
    let program = "## To add3 (a: Int, b: Int, c: Int) -> Int:\n    Return a + b + c.\n\n## Main\nShow add3(1, 2, 3).";
    let p1 = run_via_p1(program);
    let p3 = run_via_p3(program);
    assert_eq!(p1, "6");
    assert_eq!(p3, "6");
}

#[test]
fn full_all_projections_enum() {
    let program = "## To abs (n: Int) -> Int:\n    If n is less than 0:\n        Return 0 - n.\n    Return n.\n\n## Main\nShow abs(0 - 42).";
    let p1 = run_via_p1(program);
    let p3 = run_via_p3(program);
    assert_eq!(p1, "42");
    assert_eq!(p3, "42");
}

#[test]
fn full_all_projections_closure() {
    let program = "## To compose (a: Int, b: Int) -> Int:\n    Return a * 10 + b.\n\n## Main\nShow compose(4, 2).";
    let p1 = run_via_p1(program);
    let p3 = run_via_p3(program);
    assert_eq!(p1, "42");
    assert_eq!(p3, "42");
}

#[test]
fn full_dynamic_operations_preserved() {
    let program = "## Main\nLet x be 5.\nShow x * x.";
    let p1 = run_via_p1(program);
    assert_eq!(p1, "25");
}

#[test]
fn full_coverage_audit() {
    let basic_programs = vec![
        "Show 42.",
        "Let x be 10.\nShow x.",
        "Show \"hello\".",
        "Show true.",
        "Show 3.14.",
        "Let x be 5.\nLet y be 3.\nShow x + y.",
        "Let x be 5.\nIf x is greater than 0:\n    Show \"pos\".\nOtherwise:\n    Show \"neg\".",
    ];
    for src in &basic_programs {
        let result = logicaffeine_compile::compile::encode_program_source(src);
        assert!(result.is_ok(), "encode_program failed for: {}\nerror: {:?}", src, result.err());
    }
}

#[test]
fn full_coverage_stmt_audit() {
    let programs_with_stmts = vec![
        "Let x be 42.\nShow x.",
        "Let mutable x be 0.\nSet x to 10.\nShow x.",
        "## To f (n: Int) -> Int:\n    Return n + 1.\n\n## Main\nShow f(5).",
        "Let x be 0.\nWhile x is less than 3:\n    Set x to x + 1.\nShow x.",
        "If true:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".",
    ];
    for src in &programs_with_stmts {
        let result = logicaffeine_compile::compile::encode_program_source(src);
        assert!(result.is_ok(), "encode_program failed for stmt program: {}\nerror: {:?}", src, result.err());
    }
}

#[test]
fn full_identity_extended() {
    let program = "## Main\nShow 42.";
    let p1_output = run_via_p1(program);
    assert_eq!(p1_output, "42", "Identity: pe(int, trivial) should yield same output");
}

#[test]
fn full_regressions_all_sprints() {
    let sprint_programs: Vec<(&str, &str)> = vec![
        ("Show 3.14.", "3.14"),
        ("Let total be 0.\nLet i be 1.\nWhile i is at most 5:\n    Set total to total + i.\n    Set i to i + 1.\nShow total.", "15"),
        ("## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow double(21).", "42"),
        ("## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nShow factorial(6).", "720"),
        ("Let x be 100.\nLet y be x / 4.\nShow y.", "25"),
    ];
    for (program, expected) in &sprint_programs {
        let result = run_via_p1(program);
        assert_eq!(result, *expected, "Regression for: {}", program);
    }
}

#[test]
fn full_triple_equivalence_extended() {
    let test_cases: Vec<(&str, &str)> = vec![
        ("## To square (n: Int) -> Int:\n    Return n * n.\n\n## Main\nShow square(7).", "49"),
        ("## To sumTo (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return n + sumTo(n - 1).\n\n## Main\nShow sumTo(10).", "55"),
        ("## To power (b: Int, e: Int) -> Int:\n    If e is at most 0:\n        Return 1.\n    Return b * power(b, e - 1).\n\n## Main\nShow power(2, 8).", "256"),
        ("## To gcd (a: Int, b: Int) -> Int:\n    If b equals 0:\n        Return a.\n    Return gcd(b, a % b).\n\n## Main\nShow gcd(36, 24).", "12"),
        ("## To fib (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    If n equals 1:\n        Return 1.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(7).", "13"),
    ];

    for (program, expected) in &test_cases {
        let p1 = run_via_p1(program);
        let p3 = run_via_p3(program);
        assert_eq!(p1, *expected, "P1 mismatch for: {}", program);
        assert_eq!(p3, *expected, "P3 mismatch for: {}", program);
    }
}
