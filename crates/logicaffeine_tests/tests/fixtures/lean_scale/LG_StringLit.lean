/-!
Lane LG — minimal originals for Lean 4.14 string literals. Lean 4.14's `String` is the structure
`String.mk (data : List Char)`, and its kernel reads a literal as `String.mk` applied to the list
of `Char.ofNat` code points (later Lean versions, whose `String` is UTF-8 bytes, use
`String.ofList`). Each theorem is `rfl`, so it checks only if the literal means exactly that list.
-/
namespace LG_SL
theorem emptyIsMk : "" = String.mk [] := rfl
theorem abIsMk : "ab" = String.mk ['a', 'b'] := rfl
theorem unicodeIsMk : "é𝄞e\u0301" = String.mk ['é', '𝄞', 'e', '\u0301'] := rfl
def hello : String := "hello"
theorem helloData : hello.data = ['h', 'e', 'l', 'l', 'o'] := rfl
theorem helloLength : hello.length = 5 := rfl
end LG_SL
