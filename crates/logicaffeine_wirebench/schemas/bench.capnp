@0xd4c9b8a7f6e5d4c3;

# Mirrors the wirebench payload matrix: the same logical data every codec sees.
struct Point {
  x @0 :Int64;
  y @1 :Int64;
}

struct Record {
  id @0 :Int64;
  name @1 :Text;
  active @2 :Bool;
}

struct Ints {
  v @0 :List(Int64);
}

struct Points {
  items @0 :List(Point);
}

struct Records {
  items @0 :List(Record);
}

struct Strings {
  v @0 :List(Text);
}
