use super::*;

#[test]
fn transparent_type_aliases_preserve_underlying_types() {
    let path = fixture_path("transparent-aliases");
    fs::write(
        &path,
        "type Path = str;\n\
         type Paths = list<Path>;\n\
         type MaybePath = Path|nil;\n\
         val echo = fn(value:Path):str { value }\n\
         val classify = fn(value) match { _:Path => \"path\"; _ => \"other\" }\n\
         val S = struct { name:str }\n\
         type User = struct<S>;\n\
         val paths:Paths = [\"Slug\"]\n\
         val user:User = S {name: \"Nominal\"}\n\
         val missing:MaybePath = nil\n\
         println(echo(paths[0]), classify(paths[0]), user.name, missing == nil)\n",
    )
    .expect("write alias source");
    let output = slug().arg(&path).output().expect("run alias source");
    fs::remove_file(&path).expect("remove alias source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Slug path Nominal true\n"
    );
}

#[test]
fn type_aliases_reject_cycles_and_type_arguments_to_a_plain_alias() {
    let path = fixture_path("type-alias-errors");
    fs::write(&path, "type First = Second\ntype Second = First\n").expect("write cyclic aliases");
    let output = slug().arg(&path).output().expect("run cyclic aliases");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("recursive type alias: First -> Second -> First"),
        "{stderr}"
    );

    fs::write(&path, "type Path = str\nval path:Path<str> = \"Slug\"\n")
        .expect("write generic alias use");
    let output = slug().arg(&path).output().expect("run generic alias use");
    fs::remove_file(&path).expect("remove alias source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("type alias `Path` cannot accept type arguments")
    );
}

#[test]
fn generic_type_aliases_fill_in_their_parameters_at_each_use() {
    let path = fixture_path("generic-type-aliases");
    fs::write(
        &path,
        "type Optional<T> = T|nil\n\
         type Pair<T, U> = [T, U]\n\
         type Twin<T> = Pair<T, T>\n\
         type Table<V> = map<str, list<Optional<V>>>\n\
         val first = fn(pair:Pair<num, str>):num { pair[0] }\n\
         val label = fn(name:Optional<str>):str { name ?? \"anon\" }\n\
         val twins:Twin<num> = [1, 2]\n\
         val table:Table<num> = {a: [1, nil]}\n\
         val keep = fn<T>(value:Optional<T>, fallback:T):T { value ?? fallback }\n\
         val classify = fn(v) match { n: Optional<num> => \"optional\"; _ => \"other\" }\n\
         val main = fn() {\n\
           println(first([1, \"a\"]), label(nil), label(\"Ada\"), twins, table, keep(nil, 3))\n\
           println(classify(nil), classify(\"a\"))\n\
         }\n",
    )
    .expect("write generic aliases");
    let output = slug().arg(&path).output().expect("run generic aliases");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "1 anon Ada [1, 2] {\"a\": [1, nil]} 3\noptional other\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    for (source, message) in [
        ("val x:Optional<num> = \"a\"", "expected num|nil, got str"),
        (
            "val x:Optional = 1",
            "type alias `Optional` needs 1 type argument(s)",
        ),
        (
            "val x:Pair<num> = [1]",
            "type alias `Pair` takes 2 type argument(s), got 1",
        ),
        ("val x:Nope<num> = 1", "unknown type constructor `Nope`"),
        ("type Bad<T, T> = T", "duplicate type parameter `T`"),
        (
            "type Loop<T> = Loop<T>|nil",
            "recursive type alias: Loop -> Loop",
        ),
    ] {
        fs::write(
            &path,
            format!("type Optional<T> = T|nil\ntype Pair<T, U> = [T, U]\n{source}\n"),
        )
        .expect("write alias error");
        let output = slug().arg(&path).output().expect("run alias error");
        assert_eq!(output.status.code(), Some(1), "{source}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(message), "{source}: {stderr}");
    }
    fs::remove_file(&path).expect("remove alias source");
}

#[test]
fn recursive_type_aliases_are_contractive_and_regular() {
    let path = fixture_path("recursive-type-aliases");
    fs::write(
        &path,
        "type Json = nil|bool|num|str|list<Json>|map<str, Json>\n\
         type Tree<T> = [T, list<Tree<T>>]\n\
         type Even = list<Odd>|nil\n\
         type Odd = map<str, Even>\n\
         type Box<T> = list<T>\n\
         type Boxes = Box<Boxes>|nil\n\
         val doc:Json = {name: \"slug\", tags: [\"a\"], nested: {deep: [1, {x: nil}]}}\n\
         val tree:Tree<num> = [1, [[2, []], [3, [[4, []]]]]]\n\
         val even:Even = [{a: [{b: nil}]}]\n\
         val boxes:Boxes = [[nil]]\n\
         val count = fn(node:Tree<num>):num {\n\
           val go = fn(children:list<Tree<num>>, i:num, total:num):num {\n\
             if (i == len(children)) { total } else { recur(children, i + 1, total + count(children[i])) }\n\
           }\n\
           go(node[1], 0, 1)\n\
         }\n\
         val name = match doc { fields: map => fields[\"name\"]; _ => nil }\n\
         val main = fn() { println(count(tree), name, even, boxes) }\n",
    )
    .expect("write recursive aliases");
    let output = slug().arg(&path).output().expect("run recursive aliases");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "4 slug [{\"a\": [{\"b\": nil}]}] [[nil]]\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    for (source, message) in [
        (
            "type Loop = Loop|nil",
            "recursive type alias: Loop -> Loop; a reference back",
        ),
        (
            "type A = B\ntype B = A",
            "recursive type alias: A -> B -> A",
        ),
        (
            "type Opt<T> = T|nil\ntype L = Opt<L>",
            "recursive type alias: L -> Opt -> L",
        ),
        (
            "type Nest<T> = list<Nest<list<T>>>",
            "recursive type alias `Nest` must refer to itself with the same type arguments",
        ),
        (
            "type Json = nil|num|list<Json>\nval bad:Json = [1, \"x\"]",
            "expected Json, got list<num|str>",
        ),
        (
            "type Json = nil|num|list<Json>\nval f = fn(v) match { j: Json => 1; _ => 2 }",
            "match type constraint is not runtime-checkable",
        ),
    ] {
        fs::write(&path, format!("{source}\n")).expect("write recursive alias error");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run recursive alias error");
        assert_eq!(output.status.code(), Some(1), "{source}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(message), "{source}: {stderr}");
    }
    fs::remove_file(&path).expect("remove recursive alias source");
}

#[test]
fn record_types_check_maps_and_structs_by_their_fields() {
    let path = fixture_path("record-types");
    fs::write(
        &path,
        "type Named = {name: str}\n\
         type Player = {\n\
           name: str\n\
           hp: num\n\
           guild: str|nil\n\
         }\n\
         type Box<T> = {value: T}\n\
         type Tree = {value: num, children: list<Tree>}\n\
         val Pet = struct { name: str, legs: num = 4 }\n\
         val greet = fn(who:Named):str { \"hi \" + who.name }\n\
         val heal = fn(p:Player):num { p.hp + 1 }\n\
         val guild = fn(p:{guild: str|nil}):str { if (p.guild != nil) { p.guild } else { \"none\" } }\n\
         val unbox = fn<T>(box:Box<T>):T { box.value }\n\
         val p = {name: \"bo\", hp: 2, guild: nil}\n\
         val t:Tree = {value: 1, children: [{value: 2, children: []}]}\n\
         val counts:map<str, num> = {a: 1, b: 2}\n\
         val healed = p copy {hp: 9}\n\
         val {name, hp} = p + {hp: 5}\n\
         val quoted:{\"first name\": str} = {\"first name\": \"x\"}\n\
         val kind = fn(v) match {\n\
           r: {name: str} => r.name\n\
           r: {weight: num} => \"weighs ${r.weight}\"\n\
           _ => \"?\"\n\
         }\n\
         val main = fn() {\n\
           println(greet({name: \"ann\", extra: 1}), greet(Pet { name: \"rex\" }), heal(p), heal(healed))\n\
           println(guild(p), guild({guild: \"g\"}), guild({}), unbox({value: \"s\"}) + \"!\")\n\
           println(t.children[0].value, counts, name, hp, quoted)\n\
           println(kind({name: \"a\"}), kind(Pet { name: \"p\" }), kind({weight: 3}), kind({name: 1}), kind(3))\n\
         }\n",
    )
    .expect("write record types");
    let output = slug().arg(&path).output().expect("run record types");
    assert_eq!(
        String::from_utf8(output.stdout.clone()).unwrap(),
        "hi ann hi rex 3 10\n\
         none g none s!\n\
         2 {\"a\": 1, \"b\": 2} bo 5 {\"first name\": \"x\"}\n\
         a p weighs 3 ? ?\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Record match constraints survive a compiled artifact.
    let artifact = path.with_extension("cslug");
    let compile = slug()
        .args([
            "compile",
            path.to_str().unwrap(),
            artifact.to_str().unwrap(),
        ])
        .output()
        .expect("compile record types");
    assert!(compile.status.success(), "{compile:?}");
    let run = slug()
        .args(["run", artifact.to_str().unwrap()])
        .output()
        .expect("run record types artifact");
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        String::from_utf8(output.stdout).unwrap()
    );
    fs::remove_file(&artifact).expect("remove record types artifact");

    fs::remove_file(&path).expect("remove record type source");
}

#[test]
fn record_type_mismatches_name_the_field() {
    let path = fixture_path("record-type-errors");
    let prelude = "val Pet = struct { name: str }\n\
                   val greet = fn(who:{name: str}):str { who.name }\n";
    for (source, message) in [
        (
            "greet({nam: \"ann\"})",
            "expected {name: str}, got {nam: str}: field `name` is missing",
        ),
        (
            "greet({name: 3})",
            "expected {name: str}, got {name: num}: field `name` is num, but must be str",
        ),
        (
            "val m:map<str, str> = {name: \"a\"}\ngreet(m)",
            "field `name` may be missing, which reads `nil`, but must be str",
        ),
        (
            "val f = fn(p:{name: num}) { p }\nf(Pet { name: \"x\" })",
            "got struct<Pet>: field `name` is str, but must be num",
        ),
        (
            "val Rock = struct { weight: num }\ngreet(Rock { weight: 1 })",
            "the struct has no field `name`",
        ),
        (
            "val f = fn(p:{name: str}):num { p.name }",
            "expected num, got str",
        ),
        (
            "val p = {hp: 1}\nval q = p copy {hp: \"x\"}\nval f = fn(r:{hp: num}) { r }\nf(q)",
            "field `hp` is str, but must be num",
        ),
        (
            "val f = fn(p:{hp: str}) { val {hp} = p\nhp - 1 }",
            "expected num, got str",
        ),
        ("type Bad = {}", "a record type needs at least one field"),
        ("type Bad = {a: num, a: str}", "duplicate record field `a`"),
        (
            "val k = fn(v:struct<Pet>):str match { r: {name: num} => \"n\"; _ => \"p\" }",
            "match case cannot match remaining type struct<Pet>",
        ),
        (
            "val k = fn(v:struct<Pet>):str match { r: {name: str} => r.name; _ => \"p\" }",
            "match case is unreachable",
        ),
    ] {
        fs::write(&path, format!("{prelude}{source}\n")).expect("write record type error");
        let output = slug().arg(&path).output().expect("run record type error");
        assert_eq!(output.status.code(), Some(1), "{source}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(message), "{source}: {stderr}");
    }
    fs::remove_file(&path).expect("remove record type source");
}

#[test]
fn generic_schemas_infer_and_check_their_type_arguments() {
    let path = fixture_path("generic-schemas");
    fs::write(
        &path,
        "val Pair = struct<T, U> { first: T, second: U }\n\
         val Box = struct<T> {\n\
           value: T\n\
           tags: list<str> = []\n\
         }\n\
         type Opt<T> = struct<Box<T>>|nil\n\
         val p = Pair { first: 1, second: \"a\" }\n\
         val wide:struct<Pair<num, str|nil>> = p\n\
         val firstOf = fn<T, U>(pair:struct<Pair<T, U>>):T { pair.first }\n\
         val swap = fn<T, U>(pair:struct<Pair<T, U>>):struct<Pair<U, T>> {\n\
           Pair { first: pair.second, second: pair.first }\n\
         }\n\
         val b = Box { value: [1, 2] }\n\
         val raw:struct<Box> = Box { value: \"s\" }\n\
         val o:Opt<num> = Box { value: 3 }\n\
         val kind = fn(v) match { x: struct<Box> => \"box\"; _ => \"?\" }\n\
         val named = fn(r:{first: num}):num { r.first }\n\
         val {first, second} = p\n\
         val main = fn() {\n\
           println(p.first + 1, p.second + \"!\", firstOf(p) + 10, swap(p).first + \"?\")\n\
           println(b.value[0] + 1, (b copy { value: [3] }).value, kind(raw), kind(p), named(wide))\n\
           println(first + 1, second, (o ?? Box { value: 0 }).value)\n\
         }\n",
    )
    .expect("write generic schemas");
    let output = slug().arg(&path).output().expect("run generic schemas");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "2 a! 11 a?\n\
         2 [3] box ? 1\n\
         2 a 3\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let prelude = "val Pair = struct<T, U> { first: T, second: U }\n\
                   val Box = struct<T> { value: T }\n\
                   val Plain = struct { x: num }\n";
    for (source, message) in [
        (
            "val p = Pair { first: 1, second: \"a\" }\np.first + \"x\"",
            "operator `+` does not accept num and str",
        ),
        (
            "val q:struct<Pair<str, str>> = Pair { first: 1, second: \"a\" }",
            "expected struct<Pair<str, str>>, got struct<Pair<num, str>>",
        ),
        (
            "val q:struct<Pair<num>> = Pair { first: 1, second: \"a\" }",
            "schema `Pair` takes 2 type argument(s)",
        ),
        (
            "val q:struct<Plain<num>> = Plain { x: 1 }",
            "schema `Plain` takes no type arguments",
        ),
        (
            "val f = fn(v) match { x: struct<Box<num>> => 1; _ => 2 }",
            "type arguments are not checked at run time, so match `struct<Box>`",
        ),
        (
            "val b = Box { value: 1 }\nb copy { value: \"s\" }",
            "expected num, got str",
        ),
        (
            "val Bad = struct<T, T> { a: T }",
            "duplicate type parameter `T`",
        ),
        (
            "val f = fn<T>(x:T) { val S = struct<U> { a: U, b: T } }",
            "unknown type `T`",
        ),
    ] {
        fs::write(&path, format!("{prelude}{source}\n")).expect("write generic schema error");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run generic schema error");
        assert_eq!(output.status.code(), Some(1), "{source}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(message), "{source}: {stderr}");
    }
    fs::remove_file(&path).expect("remove generic schema source");
}

#[test]
fn payload_enum_cases_construct_match_and_cover() {
    let path = fixture_path("payload-enums");
    fs::write(
        &path,
        "enum Result<T, E> {\n\
           Ok { value: T }\n\
           Error { error: E }\n\
         }\n\
         enum Shape { Circle { radius: num }, Square { side: num, label: str = \"sq\" }, Empty }\n\
         val parsed:Result<num, str> = Result.Ok { value: 3 }\n\
         val failed:Result<num, str> = Result.Error { error: \"bad\" }\n\
         val unwrapOr = fn<T>(r:Result<T, str>, fallback:T):T {\n\
           match r {\n\
             {value}: Result.Ok => value\n\
             _: Result.Error => fallback\n\
           }\n\
         }\n\
         val area = fn(s:Shape):num {\n\
           match s {\n\
             {radius}: Shape.Circle => radius * radius\n\
             {side}: Shape.Square => side * side\n\
             Shape.Empty => 0\n\
           }\n\
         }\n\
         val kind = fn(v) match { _: Shape => \"shape\"; _: list<Shape> => \"shapes\"; _ => \"?\" }\n\
         val named = fn(r:{radius: num}):num { r.radius }\n\
         val circle = Shape.Circle { radius: 2 }\n\
         val maybe = fn(r:Result<num, str>|nil):num { match r { {value}: Result.Ok => value; _ => -1 } }\n\
         val main = fn() {\n\
           println(unwrapOr(parsed, 0) + 1, unwrapOr(failed, 7), parsed == Result.Ok { value: 3 }, parsed == failed)\n\
           println(area(circle), area(Shape.Square { side: 3 }), area(Shape.Empty), Shape.Square { side: 1 }.label)\n\
           println(kind(circle), kind(Shape.Empty), kind([Shape.Empty, circle]), kind(3), named(circle), (circle copy { radius: 5 }).radius, maybe(parsed), maybe(nil))\n\
         }\n",
    )
    .expect("write payload enums");
    let output = slug().arg(&path).output().expect("run payload enums");
    let expected = "4 7 true false\n\
                    4 9 0 sq\n\
                    shape shape shapes ? 2 5 3 -1\n";
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        expected,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Enum values are constants, which a compiled artifact keeps.
    let artifact = path.with_extension("cslug");
    let compile = slug()
        .args([
            "compile",
            path.to_str().unwrap(),
            artifact.to_str().unwrap(),
        ])
        .output()
        .expect("compile payload enums");
    assert!(compile.status.success(), "{compile:?}");
    let run = slug()
        .args(["run", artifact.to_str().unwrap()])
        .output()
        .expect("run payload enums artifact");
    assert_eq!(String::from_utf8(run.stdout).unwrap(), expected);
    fs::remove_file(&artifact).expect("remove payload enums artifact");

    fs::remove_file(&path).expect("remove payload enum source");
}

#[test]
fn payload_enum_mistakes_are_named() {
    let path = fixture_path("payload-enum-errors");
    let prelude = "enum Result<T, E> {\n  Ok { value: T }\n  Error { error: E }\n}\n\
                   enum Shape { Circle { radius: num }, Empty }\n";
    for (source, message) in [
        (
            "val f = fn(r:Result<num, str>):num { match r { {value}: Result.Ok => value + \"x\"; _ => 0 } }",
            "operator `+` does not accept num and str",
        ),
        (
            "val f = fn(r:Result<num, str>):num { match r { {value}: Result.Ok => value } }",
            "non-exhaustive match; missing Result.Error",
        ),
        (
            "val f = fn(s:Shape):num { match s { {radius: 1}: Shape.Circle => 1; Shape.Empty => 0 } }",
            "non-exhaustive match; missing Shape.Circle",
        ),
        (
            "val f = fn(r:Result<num, str>):num { match r { _: Result.Ok => 1; _: Result.Ok => 2; _ => 3 } }",
            "match case cannot match remaining enum cases",
        ),
        (
            "val r:Result<num, str> = Result.Ok { value: \"s\" }",
            "expected Result<num, str>, got Result.Ok<str, unknown>",
        ),
        (
            "val r:Result<num> = Result.Ok { value: 1 }",
            "enum `Result` takes 2 type argument(s)",
        ),
        (
            "val pick = fn<Left, Right>(a:Left, b:Left, c:Right) { a }\npick(1, \"s\", 2)",
            "expected num, got str (generic Left was inferred as num)",
        ),
        (
            "val f = fn<Item>(value:Item):any { value }",
            "expected any, got Item",
        ),
        (
            "val s:Shape = Result.Ok { value: 1 }",
            "expected Shape, got Result.Ok<num, unknown>",
        ),
        (
            "val c = Shape.Circle { size: 1 }",
            "struct schema has no field `size`",
        ),
        (
            "val c = Shape.Circle {}",
            "missing required struct field `radius`",
        ),
        (
            "val f = fn(v) match { x: Result<num, str> => 1; _ => 2 }",
            "type arguments are not checked at run time, so match `Result`",
        ),
    ] {
        fs::write(&path, format!("{prelude}{source}\n")).expect("write payload enum error");
        let output = slug().arg(&path).output().expect("run payload enum error");
        assert_eq!(output.status.code(), Some(1), "{source}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(message), "{source}: {stderr}");
    }
    fs::remove_file(&path).expect("remove payload enum source");
}

#[test]
fn standard_results_read_through_the_pipeline() {
    let path = fixture_path("standard-results");
    fs::write(
        &path,
        "val {Result, or, orElse, orThrow} = import(\"slug.std\")\n\
         val parse = fn(text:str):Result<num, str> {\n\
           if (text == \"\") { Result.Error { error: \"empty\" } } else { Result.Ok { value: len(text) } }\n\
         }\n\
         val name:str|nil = nil\n\
         val held:Result<str|nil, str> = Result.Ok { value: name }\n\
         val maybe:num|nil = parse(\"\") /> or(nil)\n\
         val main = fn() {\n\
           println(parse(\"abc\") /> or(0), parse(\"\") /> or(0), maybe, held /> or(\"none\"))\n\
           println(parse(\"\") /> orElse(fn(e:str):num { len(e) }))\n\
           println(parse(\"ab\") /> orThrow(fn(e) { Error {type: \"parse\", msg: e} }))\n\
           parse(\"\") /> orThrow(fn(e) { Error {type: \"parse\", msg: e} })\n\
         }\n",
    )
    .expect("write standard results");
    let output = slug().arg(&path).output().expect("run standard results");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "3 0 nil nil\n5\n2\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("\"type\": \"parse\", \"msg\": \"empty\""),
        "{stderr}"
    );

    // `nil` widens an inferred type argument, and the result says so.
    fs::write(
        &path,
        "val {Result, or} = import(\"slug.std\")\n\
         val r:Result<num, str> = Result.Error { error: \"x\" }\n\
         val n:num = r /> or(nil)\n",
    )
    .expect("write widened result");
    let output = slug().arg(&path).output().expect("run widened result");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("expected num, got num|nil")
    );
    fs::remove_file(&path).expect("remove standard results source");
}

#[test]
fn function_literals_take_parameter_types_from_where_they_are_used() {
    let path = fixture_path("expected-function-literals");
    let prelude = "val mapped = fn<T, U>(xs:list<T>, f:fn<U, T>, i:num = 0, acc:list<U> = []):list<U> {\n\
                     if (i == len(xs)) { acc } else { recur(xs, f, i + 1, acc :+ f(xs[i])) }\n\
                   }\n\
                   val apply = fn(f:fn<num, num>):num { f(3) }\n";
    fs::write(
        &path,
        format!(
            "{prelude}\
             val names = mapped([1, 2, 3], fn(n) {{ \"#\" + n }})\n\
             val lengths:list<num> = mapped(names, fn(s) {{ len(s) }})\n\
             val doubled = [1, 2] /> mapped(fn(n) {{ n * 2 }})\n\
             val inc:fn<num, num> = fn(x) {{ x + 1 }}\n\
             println(names, lengths, doubled, apply(fn(x) {{ x + 1 }}), inc(2))\n"
        ),
    )
    .expect("write expected function literals");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run expected function literals");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[\"#1\", \"#2\", \"#3\"] [2, 2, 2] [2, 4] 4 3\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (source, message) in [
        // `U` is inferred from the literal's result.
        (
            "val r:list<str> = mapped([1, 2], fn(n) { n * 2 })",
            "expected str, got num (generic U was inferred as str)",
        ),
        // `s` is a `str`, from the list.
        ("mapped([\"a\"], fn(s) { s - 2 })", "expected num, got str"),
        (
            "apply(fn(x) { x + \"!\" })",
            "operator `+` does not accept num and str",
        ),
        (
            "val inc:fn<num, num> = fn(x) { x + 1 }\ninc(\"s\")",
            "expected num, got str",
        ),
    ] {
        fs::write(&path, format!("{prelude}{source}\n")).expect("write literal error");
        let output = slug().arg(&path).output().expect("run literal error");
        assert_eq!(output.status.code(), Some(1), "{source}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(message), "{source}: {stderr}");
    }
    fs::remove_file(&path).expect("remove expected function literal source");
}

#[test]
fn a_match_body_binds_its_arguments_with_their_types() {
    let path = fixture_path("typed-match-body");
    let prelude = "val mapped = fn<T, U>(xs:list<T>, f:fn<U, T>, acc:list<U> = []):list<U> match {\n\
                     [[], ...] => acc\n\
                     [[h, ...t], ...] => recur(t, f, acc :+ f(h))\n\
                   }\n";
    fs::write(
        &path,
        format!(
            "{prelude}\
             val names = mapped([1, 2, 3], fn(n) {{ \"#\" + n }})\n\
             println(names, mapped(names, fn(s) {{ len(s) }}))\n"
        ),
    )
    .expect("write typed match body");
    let output = slug().arg(&path).output().expect("run typed match body");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[\"#1\", \"#2\", \"#3\"] [2, 2, 2]\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(
        &path,
        "val t = fn(xs:list<str>, n:num):num match {\n  [[h, ...rest], m] => h - m\n  _ => 0\n}\n",
    )
    .expect("write mistyped match body");
    let output = slug().arg(&path).output().expect("run mistyped match body");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("expected num, got str")
    );
    fs::remove_file(&path).expect("remove typed match body source");
}

#[test]
fn an_annotation_on_a_call_fills_in_what_its_arguments_leave_open() {
    let path = fixture_path("expected-call-results");
    fs::write(
        &path,
        "val {reduce, find} = import(\"slug.std\")\n\
         val names:list<str> = reduce([1, 2], [], fn(acc, n) { acc :+ (\"#\" + n) })\n\
         val lengths:list<num> = [\"a\", \"bb\"] /> reduce([], fn(acc, s) { acc :+ len(s) })\n\
         val found:num|nil = find([nil, 3, 4], fn(v) { v != nil && v > 3 })\n\
         println(names, lengths, found, int(3.7) + 1)\n",
    )
    .expect("write expected call results");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run expected call results");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[\"#1\", \"#2\"] [1, 2] 4 4\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (source, message) in [
        // The accumulator is a `list<str>`, from the annotation, not `[]`.
        (
            "val {reduce} = import(\"slug.std\")\nval names:list<str> = reduce([1, 2], [], fn(acc, n) { acc :+ n })",
            "expected list<str>, got list<num|str> (generic A was inferred as list<str>)",
        ),
        (
            "val n = int(3) + \"x\"",
            "operator `+` does not accept num and str",
        ),
    ] {
        fs::write(&path, format!("{source}\n")).expect("write expected call error");
        let output = slug().arg(&path).output().expect("run expected call error");
        assert_eq!(output.status.code(), Some(1), "{source}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(message), "{source}: {stderr}");
    }
    fs::remove_file(&path).expect("remove expected call result source");
}

#[test]
fn a_type_argument_list_at_the_end_of_a_line_ends_its_statement() {
    // A generic alias's closing `>` ends a line as `list<num>`'s does, while
    // a trailing comparison still continues onto the next line.
    let path = fixture_path("alias-line-end");
    fs::write(
        &path,
        "type Pair<T, U> = [T, U]\n\
         type Twin<T> = Pair<T, T>\n\
         val twins:Twin<num> = [1, 2]\n\
         val more = 3 >\n\
           2\n\
         println(twins, more)\n",
    )
    .expect("write alias line ends");
    let output = slug().arg(&path).output().expect("run alias line ends");
    fs::remove_file(&path).expect("remove alias line ends");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[1, 2] true\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fieldless_enum_values_are_qualified_nominal_values() {
    let path = fixture_path("fieldless-enums");
    fs::write(
        &path,
        "enum SeekFrom {\n\
           Start,\n\
           Current,\n\
           End,\n\
         }\n\
         val from:SeekFrom = SeekFrom.Start\n\
         println(from == SeekFrom.Start, from == SeekFrom.End)\n\
         println(match from {\n\
           SeekFrom.Start => \"start\"\n\
           SeekFrom.Current => \"current\"\n\
           SeekFrom.End => \"end\"\n\
         })\n",
    )
    .expect("write enum source");
    let output = slug().arg(&path).output().expect("run enum source");
    fs::remove_file(&path).expect("remove enum source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "true false\nstart\n"
    );
}

#[test]
fn type_check_requires_unguarded_coverage_of_each_enum_case() {
    let path = fixture_path("enum-coverage");
    fs::write(
        &path,
        "enum SeekFrom { Start, End }\n\
         val from:SeekFrom = SeekFrom.Start\n\
         match from {\n\
           SeekFrom.Start => 0\n\
           SeekFrom.End => 1\n\
         }\n",
    )
    .expect("write exhaustive enum source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run exhaustive enum source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    fs::write(
        &path,
        "enum SeekFrom { Start, End }\n\
         val from:SeekFrom = SeekFrom.Start\n\
         match from { SeekFrom.Start => 0 }\n",
    )
    .expect("write non-exhaustive enum source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run non-exhaustive enum source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("missing SeekFrom.End")
    );

    fs::write(
        &path,
        "enum SeekFrom { Start }\n\
         enum Origin { Start }\n\
         val from:SeekFrom = SeekFrom.Start\n\
         match from { Origin.Start => 0 }\n",
    )
    .expect("write mismatched enum source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run mismatched enum source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("cannot match remaining enum cases")
    );

    fs::write(
        &path,
        "enum SeekFrom { Start, End }\n\
         val from:SeekFrom = SeekFrom.Start\n\
         match from {\n\
           SeekFrom.Start if true => 0\n\
           SeekFrom.End => 1\n\
         }\n",
    )
    .expect("write guarded enum source");
    let output = slug().arg(&path).output().expect("run guarded enum source");
    fs::remove_file(path).expect("remove enum source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("missing SeekFrom.Start")
    );
}

#[test]
fn pipes_values_into_calls_and_subjectless_matches() {
    let path = fixture_path("pipeline");
    fs::write(
        &path,
        "val add = fn(first, second) { first + second }\nval double = fn(value) { value * 2 }\nval total = 2 /> add(3) /> double\nval first = [1, 2, 3] /> match {\n  [head, ...] => head\n}\nprintln(total, first)\n",
    )
    .expect("write pipeline source");
    let output = slug().arg(&path).output().expect("run pipeline source");
    fs::remove_file(&path).expect("remove pipeline source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "10 1\n");

    fs::write(&path, "1 /> match 2 { _ => 3 }\n").expect("write invalid pipeline match source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run invalid pipeline match source");
    fs::remove_file(path).expect("remove invalid pipeline match source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("slug: semantic error: pipeline match must omit its subject")
    );
}

#[test]
fn matches_and_destructures_structs_by_schema_identity() {
    let path = fixture_path("struct-patterns");
    fs::write(
        &path,
        "val User = struct { name, active = true }\nval OtherUser = struct { name, active = true }\nval user = User { name: \"Slug\" }\nval describe = fn(value) match {\n  {name, active: true}: struct<User> => name\n  _ => \"other\"\n}\nval missing = fn(value) match {\n  {missing}: struct<User> => \"matched\"\n  _ => \"other\"\n}\nprintln(describe(user), describe(OtherUser { name: \"Slug\" }), missing(user), user.name)\n",
    )
    .expect("write struct pattern source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run struct pattern source");
    fs::remove_file(&path).expect("remove struct pattern source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Slug other other Slug\n"
    );

    fs::write(
        &path,
        "val User = struct { name }\nmatch User { name: \"Slug\" } { {name, name}: struct<User> => name }\n",
    )
    .expect("write duplicate struct pattern source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run duplicate struct pattern source");
    fs::remove_file(&path).expect("remove duplicate struct pattern source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("slug: parse error: duplicate map pattern key `name`")
    );

    fs::write(
        &path,
        "val NotSchema = 1\nmatch (nil) { _: struct<NotSchema> => true }\n",
    )
    .expect("write invalid struct pattern schema source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run invalid struct pattern schema source");
    fs::remove_file(path).expect("remove invalid struct pattern schema source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("slug:")
    );
}

#[test]
fn matches_reifiable_type_constraints_and_narrows_case_bindings() {
    let path = fixture_path("match-type-constraints");
    fs::write(
        &path,
        "val Marker = struct {}\n\
         val classify = fn(value) match {\n\
           b: bool => if (b) { \"true\" } else { \"false\" }\n\
           {|first, second|}: map<str, str> => first + second\n\
           [head, ...]: list<num> => head\n\
           _: str|bytes => \"text\"\n\
           _: struct => \"struct\"\n\
           _: any => \"other\"\n\
           _ => \"nil\"\n\
         }\n\
         val takesBool = fn(value:bool) { value }\n\
         val narrowed = fn(value:any|nil) match {\n\
           b: bool => takesBool(b)\n\
           _ => false\n\
         }\n\
         val piped = true /> match { b: bool => b; _ => false }\n\
         println(classify(true), classify({first: \"a\", second: \"b\"}), classify({first: \"a\", second: 2}), classify([7, 8]), classify([true]), classify(\"Slug\"), classify(Marker {}), classify(1), classify(nil), narrowed(true), piped)\n",
    )
    .expect("write match type constraint source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run match type constraint source");
    fs::remove_file(path).expect("remove match type constraint source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "true ab other 7 other text struct other nil true true\n"
    );
}

#[test]
fn distinguishes_schema_values_from_struct_instances_and_infers_nominal_types() {
    let path = fixture_path("schema-types");
    fs::write(
        &path,
        "val S: schema = struct { name: str }\n\
         val Alias = S\n\
         val s: struct<S> = S {name: \"evan\"}\n\
         val alias: struct<Alias> = Alias {name: \"ada\"}\n\
         val takesSchema = fn(value: schema) { \"narrowed\" }\n\
         val classify = fn(value) match {\n\
           _: schema => \"schema\"\n\
           _: struct => \"struct\"\n\
           _ => \"other\"\n\
         }\n\
         val narrowed = fn(value) match { found: schema => takesSchema(found); _ => \"other\" }\n\
         println(classify(S), classify(s), classify(alias), narrowed(S))\n",
    )
    .expect("write schema type source");
    let output = slug().arg(&path).output().expect("run schema type source");
    fs::remove_file(&path).expect("remove schema type source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "schema struct struct narrowed\n"
    );

    fs::write(
        &path,
        "val S = struct {}\nval Other = struct {}\nval value: struct<S> = Other {}\n",
    )
    .expect("write mismatched nominal struct source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run mismatched nominal struct source");
    fs::remove_file(&path).expect("remove mismatched nominal struct source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("slug: semantic error: expected struct<S>, got struct<Other>")
    );

    fs::write(&path, "val S: schema<num> = struct {}\n")
        .expect("write parameterized schema source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run parameterized schema source");
    fs::remove_file(path).expect("remove parameterized schema source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("slug: semantic error: type `schema` expects 0 argument(s)")
    );
}

#[test]
fn type_check_validates_known_operations_and_preserves_collection_results() {
    let path = fixture_path("checked-expression-operations");
    fs::write(
        &path,
        "val numbers:list<num> = [1, 2]\n\
         val labels:map<str, str> = {first: \"Slug\"}\n\
         val item:num = numbers[0]\n\
         val maybe:str|nil = labels.first\n\
         val slice:list<num> = numbers[0:1]\n\
         val letter:str = \"Slug\"[3]\n\
         val textSlice:str = \"Slug\"[1:3]\n\
         val joined:list<num> = numbers + [3]\n\
         val appended:list<num> = joined :+ 4\n\
         val prepended:list<num> = 0 +: appended\n\
         val repeated:str = \"go\" * 2\n\
         val rendered:str = \"list of two + \" + len(numbers)\n\
         println(item, maybe, slice, letter, textSlice, prepended, repeated, rendered)\n",
    )
    .expect("write checked expression source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run checked expression source");
    fs::remove_file(&path).expect("remove checked expression source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "1 Slug [1] g lu [0, 1, 2, 3, 4] gogo list of two + 2\n"
    );

    for (source, expected) in [
        ("val value = \"name\" - 1\n", "expected num, got str"),
        ("val value = [1][\"first\"]\n", "expected num, got str"),
        ("val value = {name: 1}[true]\n", "expected str, got bool"),
        (
            "val value = 1[0]\n",
            "operator `[]` does not accept num and num",
        ),
        ("val value = 1[0:1]\n", "expected list, got num"),
    ] {
        fs::write(&path, source).expect("write invalid checked expression source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run invalid checked expression source");
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .starts_with(&format!("slug: semantic error: {expected}"))
        );
    }
    fs::remove_file(path).expect("remove invalid checked expression source");
}

#[test]
fn type_check_preserves_static_map_members_across_dot_chains() {
    let path = fixture_path("checked-map-member-chains");
    fs::write(
        &path,
        "val times_two = fn(value) { value * 2 }\n\
         val m1 = {key: times_two}\n\
         val m2 = {key: m1}\n\
         val m3 = {key: m2}\n\
         println(10 /> m1.key, 10 /> m2.key.key, 10 /> m3.key.key.key)\n",
    )
    .expect("write checked map member chain source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run checked map member chain source");
    fs::remove_file(path).expect("remove checked map member chain source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "20 20 20\n");
}

#[test]
fn type_check_allows_strings_to_concatenate_lists_and_maps() {
    let path = fixture_path("string-collection-concatenation");
    fs::write(
        &path,
        "val listText:str = \"items: \" + [1, 2]\nval mapText:str = \"data: \" + {status: \"ok\"}\nprintln(listText, mapText)\n",
    )
    .expect("write string collection concatenation source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run string collection concatenation source");
    fs::remove_file(path).expect("remove string collection concatenation source");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "items: [1, 2] data: {\"status\": \"ok\"}\n"
    );
}

#[test]
fn type_check_narrows_nilable_bindings_through_conditions() {
    let path = fixture_path("nil-control-flow-narrowing");
    fs::write(
        &path,
        "val use = fn(value:str):str { value + \"!\" }\n\
         val describe = fn(value:str|nil) {\n\
           if (value != nil) { use(value) } else { \"missing\" }\n\
         }\n\
         val describeEqual = fn(value:str|nil) {\n\
           if (value == nil) { \"missing\" } else { use(value) }\n\
         }\n\
         val reversedNotEqual = fn(value:str|nil) { if (nil != value) { use(value) } else { \"missing\" } }\n\
         val reversedEqual = fn(value:str|nil) { if (nil == value) { \"missing\" } else { use(value) } }\n\
         val andResult = fn(value:str|nil) { (value != nil) && use(value) }\n\
         val orResult = fn(value:str|nil) { (value == nil) || use(value) }\n\
         val composed = fn(value:str|nil) { (value != nil) && ((value != nil) && use(value)) }\n\
         val guarded = fn(value:str|nil) match { candidate if candidate != nil => use(candidate); _ => \"missing\" }\n\
         val nested = fn(value:str|nil) { if (value != nil) { if (value != nil) { use(value) } else { \"impossible\" } } else { \"missing\" } }\n\
         val shadowed = fn(value:str|nil) { if (value != nil) { val value:str = \"shadow\"; use(value) } else { \"missing\" } }\n\
         val assigned = fn(flag:bool) {\n\
           var value:str|nil = nil\n\
           if (flag) { value = \"left\" } else { value = \"right\" }\n\
           use(value)\n\
         }\n\
         println(describe(\"Slug\"), describe(nil), describeEqual(\"Slug\"), reversedNotEqual(\"reverse\"), reversedEqual(\"reverse\"), andResult(\"go\"), orResult(\"go\"), guarded(\"match\"), nested(\"nested\"), shadowed(\"outer\"), assigned(true))\n",
    )
    .expect("write nil narrowing source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run nil narrowing source");
    fs::remove_file(&path).expect("remove nil narrowing source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Slug! missing Slug! reverse! reverse! go! go! match! nested! shadow! left!\n"
    );

    fs::write(
        &path,
        "val use = fn(value:str):str { value }\n\
         val invalid = fn(value:str|nil) {\n\
           if (value != nil) { use(value) }\n\
           use(value)\n\
         }\n",
    )
    .expect("write escaped nil narrowing source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run escaped nil narrowing source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("slug: semantic error: expected str, got str|nil")
    );

    fs::write(
        &path,
        "val use = fn(value:str):str { value }\n\
         val incomplete = fn(flag:bool) {\n\
           var value:str|nil = nil\n\
           if (flag) { value = \"only\" }\n\
           use(value)\n\
         }\n",
    )
    .expect("write incomplete type-state source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run incomplete type-state source");
    fs::remove_file(path).expect("remove nil narrowing sources");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("slug: semantic error: expected str, got str|nil")
    );
}

#[test]
fn type_check_preserves_narrowing_after_terminating_if_branches() {
    let path = fixture_path("terminating-if-narrowing");
    fs::write(
        &path,
        "val use = fn(value:str):str { value + \"!\" }\n\
         val guard = fn(value:str|nil) {\n\
           if (value == nil) { return \"missing\" }\n\
           use(value)\n\
         }\n\
         val inverse = fn(value:str|nil) {\n\
           if (value != nil) { use(value) } else { return \"missing\" }\n\
           use(value)\n\
         }\n\
         println(guard(\"Slug\"), guard(nil), inverse(\"Flow\"))\n",
    )
    .expect("write terminating-if narrowing source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run terminating-if narrowing source");
    fs::remove_file(path).expect("remove terminating-if narrowing source");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Slug! missing Flow!\n"
    );
}

#[test]
fn type_check_treats_an_if_with_two_terminating_branches_as_terminating() {
    let path = fixture_path("terminating-if-expression");
    fs::write(
        &path,
        "val needs_str = fn(value:str):str { value }\n\
         val complete = fn(flag:bool):str {\n\
           if (flag) { return \"done\" } else { throw \"failed\" }\n\
           needs_str(42)\n\
         }\n\
         println(complete(true))\n",
    )
    .expect("write terminating-if expression source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run terminating-if expression source");
    fs::remove_file(path).expect("remove terminating-if expression source");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "done\n");
}

#[test]
fn type_check_preserves_surviving_named_match_subject_facts() {
    let path = fixture_path("terminating-match-narrowing");
    fs::write(
        &path,
        "val use = fn(value:str):str { value + \"!\" }\n\
         val describe = fn(value:str|nil) {\n\
           match value { _:nil => return \"missing\"; text:str => use(text) }\n\
           use(value)\n\
         }\n\
         println(describe(\"Slug\"), describe(nil))\n",
    )
    .expect("write terminating-match narrowing source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run terminating-match narrowing source");
    fs::remove_file(path).expect("remove terminating-match narrowing source");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Slug! missing\n");
}

#[test]
fn type_check_keeps_guarded_match_facts_conservative_afterward() {
    let path = fixture_path("guarded-match-flow");
    fs::write(
        &path,
        "val use = fn(value:str):str { value }\n\
         val invalid = fn(value:str|nil) {\n\
           match value { text:str if true => use(text); _ => nil }\n\
           use(value)\n\
         }\n",
    )
    .expect("write guarded-match flow source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run guarded-match flow source");
    fs::remove_file(path).expect("remove guarded-match flow source");

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("slug: semantic error: expected str, got str|nil")
    );
}

#[test]
fn type_check_preserves_facts_through_nested_guard_and_match_flow() {
    let path = fixture_path("nested-flow-narrowing");
    fs::write(
        &path,
        "val use = fn(value:str):str { value + \"!\" }\n\
         val describe = fn(value:str|num|nil) {\n\
           if (value == nil) { return \"missing\" }\n\
           match value { text:str => use(text); number:num => \"number\" }\n\
         }\n\
         println(describe(\"Slug\"), describe(1), describe(nil))\n",
    )
    .expect("write nested-flow narrowing source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run nested-flow narrowing source");
    fs::remove_file(path).expect("remove nested-flow narrowing source");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Slug! number missing\n"
    );
}

#[test]
fn type_check_preserves_nominal_identity_when_narrowing_flow_facts() {
    let path = fixture_path("nominal-flow-narrowing");
    fs::write(
        &path,
        "val S = struct {}\n\
         val T = struct {}\n\
         val use = fn(value:struct<S>|struct<T>):str { \"ready\" }\n\
         val describe = fn(value:struct<S>|struct<T>|nil) {\n\
           if (value == nil) { return \"missing\" }\n\
           use(value)\n\
         }\n\
         println(describe(S {}), describe(T {}), describe(nil))\n",
    )
    .expect("write nominal-flow narrowing source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run nominal-flow narrowing source");
    fs::remove_file(path).expect("remove nominal-flow narrowing source");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "ready ready missing\n"
    );
}

#[test]
fn type_check_reports_closed_match_coverage_without_changing_dynamic_matches() {
    let path = fixture_path("match-coverage");
    fs::write(
        &path,
        "val S = struct {}\n\
         val T = struct {}\n\
         val classify = fn(value:str|nil) { match value { _:str => \"str\"; _:nil => \"nil\" } }\n\
         val guarded = fn(value:str|nil) { match value { _:str if true => \"guarded\"; _:str => \"str\"; _:nil => \"nil\" } }\n\
         val schema = fn(value:struct<S>|struct<T>) { match value { _:struct<S> => \"s\"; _:struct<T> => \"t\" } }\n\
         val structural = fn(values:list<num>) { match values { [head, ...] => head } }\n\
         println(classify(\"Slug\"), classify(nil), guarded(\"Slug\"), schema(S {}), schema(T {}), structural([]))\n",
    )
    .expect("write covered match source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run covered match source");
    fs::remove_file(&path).expect("remove covered match source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "str nil guarded s t nil\n"
    );

    for (source, expected) in [
        (
            "val value:str|nil = nil\nmatch value { _:str => \"str\" }\n",
            "non-exhaustive match; missing nil",
        ),
        (
            "val value:str|nil = nil\nmatch value { _:str => \"str\"; _:num => \"num\"; _:nil => \"nil\" }\n",
            "match case cannot match remaining type nil",
        ),
        (
            "val value:str|nil = nil\nmatch value { _:str => \"str\"; _:nil => \"nil\"; _ => \"never\" }\n",
            "match case is unreachable",
        ),
    ] {
        fs::write(&path, source).expect("write invalid match coverage source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run invalid match coverage source");
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .starts_with(&format!("slug: semantic error: {expected}"))
        );
    }
    fs::remove_file(path).expect("remove invalid match coverage source");
}

#[test]
fn type_check_uses_known_schema_fields_for_struct_values() {
    let path = fixture_path("schema-field-types");
    fs::write(
        &path,
        "val User = struct { name:str, age:num = 0 }\n\
         val Alias = User\n\
         val user:struct<User> = User {name: \"Slug\"}\n\
         val alias:struct<Alias> = Alias {name: \"Ada\", age: 42}\n\
         val same_schema:struct<Alias> = user\n\
         val updated:struct<User> = user copy {age: 1}\n\
         val name:str = user.name\n\
         val age:num = updated.age\n\
         { val User = struct { name:num }\n\
           val shadow_safe:str = user.name\n\
           println(shadow_safe) }\n\
         println(name, alias.name, same_schema.name, age)\n",
    )
    .expect("write typed schema field source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run typed schema field source");
    fs::remove_file(&path).expect("remove typed schema field source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Slug\nSlug Ada Slug 1\n"
    );

    for (source, expected) in [
        (
            "val User = struct { name:str }\nUser {name: 1}\n",
            "expected str, got num",
        ),
        (
            "val User = struct { name:str }\nUser {name: \"Slug\", extra: true}\n",
            "struct schema has no field `extra`",
        ),
        (
            "val User = struct { name:str }\nUser {}\n",
            "missing required struct field `name`",
        ),
        (
            "val User = struct { name:str, age:num = 0 }\nval user = User {name: \"Slug\"}\nuser copy {age: \"old\"}\n",
            "expected num, got str",
        ),
        (
            "val User = struct { name:str }\nval user = User {name: \"Slug\"}\nuser.age\n",
            "struct has no field `age`",
        ),
        (
            "val NotSchema = 1\nval value:struct<NotSchema> = nil\n",
            "struct type argument `NotSchema` must resolve to a schema binding",
        ),
        (
            "val User = struct { name:str }\nval user = User {name: \"Slug\"}\nval takes_num = fn(value:num) { value }\nmatch user { {name}: struct<User> => takes_num(name) }\n",
            "expected num, got str",
        ),
        (
            "val User = struct { name:str }\nval user = User {name: \"Slug\"}\nval takes_num = fn(value:num) { value }\nval {name} = user\ntakes_num(name)\n",
            "expected num, got str",
        ),
        (
            "val User = struct { name:str }\nval user = User {name: \"Slug\"}\nval takes_num = fn(value:num) { value }\nval check = fn(value:struct<User>) { val {name} = value; takes_num(name) }\ncheck(user)\n",
            "expected num, got str",
        ),
    ] {
        fs::write(&path, source).expect("write invalid typed schema field source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run invalid typed schema field source");
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .starts_with(&format!("slug: semantic error: {expected}"))
        );
    }
    fs::remove_file(path).expect("remove invalid typed schema field source");
}

#[test]
fn rejects_non_reifiable_match_type_constraints() {
    let path = fixture_path("invalid-match-type-constraint");
    fs::write(&path, "match value { _: fn<num, num> => true }\n")
        .expect("write invalid match type constraint source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run invalid match type constraint source");
    fs::remove_file(path).expect("remove invalid match type constraint source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("slug: semantic error: match type constraint is not runtime-checkable")
    );
}

#[test]
fn evaluates_checked_bitwise_and_shift_operators() {
    let path = fixture_path("bitwise-and-shifts");
    fs::write(
        &path,
        "println(6 & 3, 4 | 1, 6 ^ 3, ~0, 1 << 4, -8 >> 2, 0x\"ff00\" & 0x\"0ff0\", 0x\"ff\" ^ 0x\"0000\", 255 & 0x\"0ff0\", ~0x\"00ff\")\n",
    )
        .expect("write bitwise source");
    let output = slug().arg(&path).output().expect("run bitwise source");
    fs::remove_file(&path).expect("remove bitwise source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "2 5 5 -1 16 -2 0x\"0f00\" 0x\"ffff\" 0x\"0ff0\" 0x\"ff00\"\n"
    );

    for source in ["1 << -1\n", "1 << 64\n", "1.5 & 1\n", "~true\n"] {
        fs::write(&path, source).expect("write invalid bitwise source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run invalid bitwise source");
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .starts_with("slug:")
        );
    }
    fs::remove_file(path).expect("remove invalid bitwise source");
}

#[test]
fn appends_and_prepends_list_values_with_checked_operands() {
    let path = fixture_path("list-concatenation");
    fs::write(
        &path,
        "val original = [1, 2]\nval appended = original :+ 3\nval combined = original + [3, 4]\nprintln(original, appended, 0 +: original, combined, len(0x\"0102\" + 0x\"0304\"), 1 +: 0x\"0203\" + 0x\"04\" :+ 5, 0x\"01\" +: 0x\"0203\" + 0x\"04\" :+ 0x\"05\", 1 +: 2 +: 3 +: 0x\"\")\n",
    )
        .expect("write list concatenation source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run list concatenation source");
    fs::remove_file(&path).expect("remove list concatenation source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[1, 2] [1, 2, 3] [0, 1, 2] [1, 2, 3, 4] 4 0x\"0102030405\" 0x\"0102030405\" 0x\"010203\"\n"
    );

    fs::write(&path, "1 :+ 2\n").expect("write invalid list concatenation source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run invalid list concatenation source");
    fs::remove_file(path).expect("remove invalid list concatenation source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("slug: semantic error:")
    );
}

#[test]
fn collection_updates_preserve_local_and_closure_aliases() {
    let path = fixture_path("collection-aliases");
    fs::write(
        &path,
        "val User = struct { name, active = true }\n\
         val list = [1, 2]\n\
         val bytes = 0x\"0203\"\n\
         val map = {stable: 1, replace: 2}\n\
         val user = User { name: \"Slug\" }\n\
         val captureList = fn() { list }\n\
         val captureBytes = fn() { bytes }\n\
         val captureMap = fn() { map }\n\
         val captureUser = fn() { user }\n\
         val appended = list :+ 3\n\
         val prepended = 0 +: list\n\
         val changedBytes = 1 +: bytes :+ 4\n\
         val merged = map + {replace: 20, added: 3}\n\
         val removed = map - \"replace\"\n\
         val copied = map copy {replace: 30, added: 4}\n\
         val changedUser = user copy {active: false}\n\
         println(list, appended, prepended, bytes, changedBytes, map.replace, merged.replace, removed.replace, copied.replace, user.active, changedUser.active, captureList(), captureBytes(), captureMap().replace, captureUser().active)\n",
    )
    .expect("write collection alias source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run collection alias source");
    fs::remove_file(path).expect("remove collection alias source");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "[1, 2] [1, 2, 3] [0, 1, 2] 0x\"0203\" 0x\"01020304\" 2 20 nil 30 true false [1, 2] 0x\"0203\" 2 true\n"
    );
}

#[test]
fn list_rest_views_preserve_aliases_through_closures_and_updates() {
    let path = fixture_path("list-rest-view-aliases");
    fs::write(
        &path,
        "val original = [1, 2, 3, 4]\n\
         val [head, ...tail] = original\n\
         val capture = fn() { tail }\n\
         val changed = tail :+ 5\n\
         val sliced = original[1:3]\n\
         println(head, tail, capture(), changed, sliced, tail == [2, 3, 4], tail[1])\n",
    )
    .expect("write list rest-view alias source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run list rest-view alias source");
    fs::remove_file(path).expect("remove list rest-view alias source");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "1 [2, 3, 4] [2, 3, 4] [2, 3, 4, 5] [2, 3] true 3\n"
    );
}

#[test]
fn persistently_merges_removes_and_enumerates_maps() {
    let path = fixture_path("persistent-map-updates");
    fs::write(
        &path,
        "val original = {first: 1, second: 2}\n\
         val merged = original + {second: 20, third: 3}\n\
         val removed = merged - \"second\"\n\
         val repeated = merged + {second: 30}\n\
         println(original.second, merged.first, merged.second, merged.third, removed.second, repeated.second, merged == {third: 3, first: 1, second: 20}, len(keys(merged)), len(keys(removed)), len(original))\n",
    )
    .expect("write persistent map update source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run persistent map update source");
    fs::remove_file(path).expect("remove persistent map update source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "2 1 20 3 nil 30 true 3 2 2\n"
    );
}

#[test]
fn string_concatenation_never_changes_a_retained_string() {
    let path = fixture_path("string-append-aliasing");
    fs::write(
        &path,
        "val base = \"a\"\n\
         val build = fn(n, text, kept) { if (n == 0) { [text, kept] } else { recur(n - 1, text + \"b\", text) } }\n\
         val [text, kept] = build(3, base, \"\")\n\
         val number = 1\n\
         println(base, text, kept, base + number + true, base)\n",
    )
    .expect("write string aliasing source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run string aliasing source");
    fs::remove_file(path).expect("remove string aliasing source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "a abbb abb a1true a\n"
    );
}

#[test]
fn fused_local_constant_operations_match_their_standalone_forms() {
    let path = fixture_path("fused-local-constant");
    let run = |source: &str| {
        fs::write(&path, source).expect("write fused operation source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run fused operation source");
        (
            output.status.code(),
            String::from_utf8(output.stdout).expect("stdout is UTF-8"),
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        )
    };
    let (code, stdout, stderr) = run(
        "val numbers = fn(x) { [x + 1, x - 1, x * 2, x / 2, x % 3, x == 4, x > 1, x < 1] }\n\
         val generic = fn(x) { [x + \"!\", x == \"a\"] }\n\
         val drop = fn(m: map) { m - \"a\" }\n\
         val build = fn(n, text) { if (n == 0) { text } else { recur(n - 1, text + \"ab\") } }\n\
         println(numbers(4), numbers(4.5))\n\
         println(generic(\"a\"), drop({a: 1, b: 2}), len(build(1000, \"\")))\n",
    );
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        stdout,
        "[5, 3, 8, 2, 1, true, true, false] [5.5, 3.5, 9, 2.25, 1.5, false, true, false]\n\
         [\"a!\", true] {\"b\": 2} 2000\n"
    );

    // A fused operation fails exactly where its operator is.
    let (code, _, stderr) = run("val generic = fn(x) { x + \"!\" }\nprintln(generic(1))\n");
    fs::remove_file(&path).expect("remove fused operation source");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("runtime error: expected numbers, got num and str"),
        "{stderr}"
    );
    assert!(stderr.contains(":1:25"), "{stderr}");
}

#[test]
fn full_positional_calls_skip_defaults_and_pipelines_prepend_their_value() {
    let path = fixture_path("positional-defaults");
    fs::write(
        &path,
        "val f = fn(a, b = 10, c = a + b) { [a, b, c] }\n\
         val loop = fn(n, acc = []) { if (n == 0) { acc } else { recur(n - 1, acc :+ n) } }\n\
         val v = fn(a, ...rest) { [a, rest] }\n\
         println(f(1), f(1, 2), f(1, 2, 3), loop(3), loop(2, [9]))\n\
         println(v(1), v(1, 2), 5 /> f(), 5 /> f(6, 7), 1 /> v(2, 3))\n",
    )
    .expect("write positional defaults source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run positional defaults source");
    fs::remove_file(path).expect("remove positional defaults source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "[1, 10, 11] [1, 2, 3] [1, 2, 3] [3, 2, 1] [9, 2, 1]\n\
         [1, []] [1, [2]] [5, 10, 15] [5, 6, 7] [1, [2, 3]]\n"
    );
}

#[test]
fn number_bytes_and_boolean_literals_are_map_keys() {
    let path = fixture_path("literal-map-keys");
    fs::write(
        &path,
        "val reasons = {200: \"OK\", 404: \"Not Found\"}\n\
         val mixed = {-1: \"negative\", 1.5: \"float\", 0x\"00\": \"bytes\", true: \"yes\", false: \"no\"}\n\
         val named = {nil: 1, copy: 2}\n\
         val found = match ({201: \"Created\", true: \"t\"}) {\n\
           {201: reason, true: flag} => reason + flag\n\
           _ => \"none\"\n\
         }\n\
         println(reasons[404], mixed[-1], mixed[1.5], mixed[0x\"00\"], mixed[true], mixed[false], named.nil, found)\n\
         println({ -1 })\n",
    )
    .expect("write literal key source");
    let output = slug().arg(&path).output().expect("run literal key source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Not Found negative float bytes yes no 1 Createdt\n-1\n"
    );

    fs::write(&path, "val m = {1: 1}\nmatch m { {1} => 1; _ => 0 }\n")
        .expect("write shorthand source");
    let output = slug().arg(&path).output().expect("run shorthand source");
    fs::remove_file(&path).expect("remove literal key source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("a literal map pattern key needs a pattern"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn enum_values_are_map_keys() {
    let path = fixture_path("enum-map-keys");
    fs::write(
        &path,
        "enum Color { Red, Green, Blue }\n\
         val hex = {[Color.Red]: \"#f00\", [Color.Blue]: \"#00f\"}\n\
         // Past the size where a map indexes its keys by hash.\n\
         val fill = fn(i:num, m:map):map { if (i == 100) { m } else { recur(i + 1, m + {[i]: i}) } }\n\
         val large = fill(0, {}) + {[Color.Green]: \"green\"}\n\
         println(hex[Color.Red], hex[Color.Green], keys(hex), large[Color.Green], len(large - Color.Green))\n",
    )
    .expect("write enum key source");
    let output = slug().arg(&path).output().expect("run enum key source");
    fs::remove_file(&path).expect("remove enum key source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "#f00 nil [Color.Red, Color.Blue] green 100\n"
    );
}

#[test]
fn lists_structs_and_channels_are_map_keys() {
    let path = fixture_path("composite-map-keys");
    fs::write(
        &path,
        "val grid = {[[0, 0]]: \"origin\", [[1, 2]]: \"a\"}\n\
         val Point = struct { x, y }\n\
         val Other = struct { x, y }\n\
         val points = {[Point {x: 1, y: 2}]: \"p\"}\n\
         val lookup = fn(key) { points[key] }\n\
         val ready = chan(1)\n\
         val channels = {[ready]: \"ready\"}\n\
         val fill = fn(i:num, m:map):map { if (i == 100) { m } else { recur(i + 1, m + {[[i, \"x\"]]: i}) } }\n\
         val large = fill(0, {})\n\
         println(grid[[0, 0]], grid[[1.0, 2]], grid[[2, 1]], lookup(Point {x: 1, y: 2}), lookup(Other {x: 1, y: 2}))\n\
         println(channels[ready], channels[chan(1)], large[[42, \"x\"]], len(large - [3, \"x\"]))\n",
    )
    .expect("write composite key source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run composite key source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "origin a nil p nil\nready nil 42 99\n"
    );

    for (source, message) in [
        (
            "{[[1, nil]]: 1}",
            "a list containing nil cannot be used as a map key",
        ),
        (
            "{[[1, [2, {}]]]: 1}",
            "a list containing map cannot be used as a map key",
        ),
        ("{[{}]: 1}", "map cannot be used as a map key"),
    ] {
        fs::write(&path, format!("println({source})\n")).expect("write invalid key source");
        let output = slug().arg(&path).output().expect("run invalid key source");
        assert_eq!(output.status.code(), Some(1), "{source}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::remove_file(&path).expect("remove composite key source");
}

#[test]
fn map_literals_reject_a_repeated_key() {
    let path = fixture_path("map-literal-repeated-keys");
    let run = |source: &str| {
        fs::write(&path, source).expect("write repeated-key source");
        let output = slug().arg(&path).output().expect("run repeated-key source");
        (
            output.status.code(),
            String::from_utf8(output.stdout).expect("stdout is UTF-8"),
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        )
    };

    // Constant keys are checked at compile time, under key equivalence.
    for (source, key) in [
        ("println({a: 1, \"b\": 2, a: 3})\n", "a"),
        ("println({[1]: \"int\", [1.0]: \"float\"})\n", "1"),
    ] {
        let (code, stdout, stderr) = run(source);
        assert_eq!(code, Some(1), "{source}");
        assert_eq!(stdout, "", "{source}");
        assert!(
            stderr.contains(&format!("map literal repeats key `{key}`")),
            "{stderr}"
        );
    }

    // A computed key can only be checked when the literal is built.
    let (code, stdout, stderr) =
        run("val key = \"a\"\nprintln(\"before\")\nprintln({[key]: 1, a: 2})\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "before\n");
    assert!(stderr.contains("map literal repeats key \"a\""), "{stderr}");

    // Distinct keys, and merge, are unaffected: merge still overwrites.
    let (code, stdout, stderr) = run("val key = \"c\"\n\
         val map = {a: 1, [key]: 2, [1]: 3, [1.5]: 4}\n\
         println(len(map), map + {a: 9} == {a: 9, c: 2, [1.0]: 3, [1.5]: 4})\n");
    fs::remove_file(&path).expect("remove repeated-key source");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "4 true\n");
}

#[test]
fn standard_clutch_preserves_non_string_map_keys() {
    let path = fixture_path("standard-clutch-map-keys");
    fs::write(
        &path,
        "val values = {[1]: \"one\", [true]: \"yes\", name: \"Slug\", [0x\"01\"]: \"byte\"}\n\
         val updated = values + {[1.0]: \"one-point-zero\"}\n\
         val missingRemoved = updated - \"missing\"\n\
         println(updated[1], updated[1.0], updated[true], updated.name, updated[0x\"01\"], updated[\"missing\"], missingRemoved == updated, len(keys(updated)))\n",
    )
    .expect("write standard clutch source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run standard clutch source");
    fs::remove_file(path).expect("remove standard clutch source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "one-point-zero one-point-zero yes Slug byte nil true 4\n"
    );
}

#[test]
fn rejects_every_non_key_collection_value_from_source() {
    let path = fixture_path("invalid-map-key-categories");
    for (name, source, expected) in [
        ("nil", "{[nil]: 1}\n", "nil cannot be used as a map key"),
        (
            "list",
            "{[[nil]]: 1}\n",
            "a list containing nil cannot be used as a map key",
        ),
        ("map", "{[{}]: 1}\n", "map cannot be used as a map key"),
        (
            "struct",
            "val User = struct { tags = {} }\n{[User {}]: 1}\n",
            "a struct containing map cannot be used as a map key",
        ),
    ] {
        fs::write(&path, source).expect("write invalid map-key source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run invalid map-key source");
        assert_eq!(output.status.code(), Some(1), "{name}");
        assert!(output.stdout.is_empty(), "{name}");
        assert!(
            String::from_utf8(output.stderr)
                .expect("stderr is UTF-8")
                .starts_with(&format!("slug: runtime error: {expected}")),
            "{name}"
        );
    }
    fs::remove_file(path).expect("remove invalid map-key source");
}

#[test]
fn parses_multiline_maps_with_raw_quoted_keys() {
    let path = fixture_path("multiline-raw-quoted-map");
    fs::write(
        &path,
        "val m = {\n\
         'k1': 1,\n\
         'k2': 2\n\
         }\n\
         println(m)\n\
         println(m.k1, m.k2, len(keys(m)))\n",
    )
    .expect("write multiline map source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run multiline map source");
    fs::remove_file(path).expect("remove multiline map source");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let mut lines = stdout.lines();
    let display = lines.next().expect("map display line");
    assert!(
        display.starts_with('{') && display.ends_with('}'),
        "{display}"
    );
    assert!(display.contains("\"k1\": 1"), "{display}");
    assert!(display.contains("\"k2\": 2"), "{display}");
    assert_eq!(lines.next(), Some("1 2 2"));
    assert_eq!(lines.next(), None);
    assert!(output.stderr.is_empty());
}

#[test]
fn persistently_copies_maps_with_string_key_updates() {
    let path = fixture_path("map-copy");
    fs::write(
        &path,
        "val opts = {timeout: 1000, retries: 2}\n\
         val next = opts copy {timeout: 5000, retries: 3, mode: \"fast\"}\n\
         println(opts.timeout, opts.retries, opts.mode, next.timeout, next.retries, next.mode)\n",
    )
    .expect("write map copy source");
    let output = slug().arg(&path).output().expect("run map copy source");
    fs::remove_file(path).expect("remove map copy source");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "1000 2 nil 5000 3 fast\n"
    );
}

#[test]
fn parses_decimal_hexadecimal_and_byte_literals() {
    let path = fixture_path("numeric-and-byte-literals");
    fs::write(
        &path,
        "val bytes = 0x\"020304\"\nprintln(1_000, 1.5, 2e3, 1.25e-2, 0x10, 0x_ff, 0x\"414243\", len(0x\"\"), bytes[0], bytes[1:])\n",
    )
    .expect("write numeric and byte literals");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run numeric and byte literals");
    fs::remove_file(path).expect("remove numeric and byte literals");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "1000 1.5 2000 0.0125 16 255 0x\"414243\" 0 2 0x\"0304\"\n"
    );
}

#[test]
fn rejects_malformed_hexadecimal_and_byte_literals_with_locations() {
    for (kind, source, message) in [
        ("empty-hex", "0x\n", "expected hexadecimal digit"),
        (
            "odd-byte-literal",
            "0x\"f\"\n",
            "byte literal must contain complete hexadecimal byte pairs",
        ),
        (
            "invalid-byte-literal",
            "0x\"gg\"\n",
            "invalid hexadecimal digit in byte literal",
        ),
        (
            "double-decimal-separator",
            "1__000\n",
            "invalid number separator",
        ),
        (
            "trailing-hexadecimal-separator",
            "0xff_\n",
            "invalid hexadecimal number separator",
        ),
        ("missing-exponent", "1e\n", "expected exponent digit"),
    ] {
        let path = fixture_path(kind);
        fs::write(&path, source).expect("write malformed literal source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run malformed literal source");
        fs::remove_file(&path).expect("remove malformed literal source");

        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
        assert!(stderr.starts_with("slug: parse error:"), "{stderr}");
        assert!(stderr.contains(message), "{stderr}");
        assert!(
            stderr.contains(&format!("    --> {}:1:1\n", path.display())),
            "{stderr}"
        );
    }
}

#[test]
fn keys_is_an_implicit_builtin_and_not_a_slug_std_export() {
    let path = fixture_path("keys-builtin");
    fs::write(
        &path,
        "val std = import(\"slug.std\")\n\
         val mixed = {name: \"Slug\", [7]: \"seven\", [true]: \"yes\"}\n\
         val pair = fn(values) { match values { [\"a\", \"b\"] => \"a and b\"; [\"b\", \"a\"] => \"a and b\"; _ => \"other\" } }\n\
         println(len(keys(mixed)), keys({}), pair(keys({b: 2, a: 1})))\n\
         println(keys({k: 1}), std.keys == nil)\n\
         val shadow = fn(keys) { keys }\n\
         println(shadow(\"local wins\"))\n",
    )
    .expect("write keys builtin source");
    let output = slug().arg(&path).output().expect("run keys builtin source");
    fs::remove_file(path).expect("remove keys builtin source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "3 [] a and b\n[\"k\"] true\nlocal wins\n"
    );
}

#[test]
fn keys_rejects_a_value_that_is_not_a_map() {
    let path = fixture_path("keys-not-a-map");
    fs::write(
        &path,
        "val dynamic = fn(value) { value }\nkeys(dynamic([1, 2]))\n",
    )
    .expect("write keys type-error source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run keys type-error source");
    fs::remove_file(path).expect("remove keys type-error source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("`keys` expects map, got List"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn strings_order_lexicographically_by_utf8_bytes() {
    let path = fixture_path("string-ordering");
    let run = |source: &str| {
        fs::write(&path, source).expect("write string ordering source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run string ordering source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    let (code, stdout, stderr) = run(
        "println(\"apple\" < \"banana\", \"b\" > \"a\", \"a\" <= \"a\", \"a\" >= \"b\", \"Z\" < \"a\", \"ab\" < \"abc\", \"é\" > \"z\")\n\
         val before = fn(a, b) { a < b }\n\
         println(before(1, 2), before(\"pear\", \"fig\"))\n\
         val small = fn(n) { n < 10 }\n\
         val early = fn(word) { word < \"m\" }\n\
         println(small(3), early(\"kiwi\"))\n",
    );
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        stdout,
        "true true true false true true true\ntrue false\ntrue true\n"
    );

    // A literal on the other side still decides an unannotated parameter's type.
    let (code, _, stderr) = run("val small = fn(n) { n < 10 }\nprintln(small(\"a\"))\n");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("semantic error: expected num, got str"),
        "{stderr}"
    );

    let (code, _, stderr) = run("println(1 < \"a\")\n");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("semantic error: operator `<` does not accept num and str"),
        "{stderr}"
    );

    let (code, _, stderr) = run("val before = fn(a, b) { a < b }\nprintln(before(1, \"a\"))\n");
    fs::remove_file(&path).expect("remove string ordering source");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("runtime error: expected two numbers or two strings, got num and str"),
        "{stderr}"
    );
}

#[test]
fn enum_cases_and_struct_fields_separate_with_commas_or_newlines() {
    let path = fixture_path("member-separators");
    let run = |source: &str| {
        fs::write(&path, source).expect("write separator source");
        let output = slug().arg(&path).output().expect("run separator source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    let (code, stdout, stderr) = run("enum Lines {\n    red\n    green\n}\n\
         enum Commas { red, green, blue }\n\
         enum Mixed {\n    red,\n    green\n    blue,\n}\n\
         val Point = struct {\n    x: num\n    y: num = 2\n}\n\
         val Pair = struct { left: num, right: num }\n\
         val S:schema = struct{\n    name:str,\n    age:num = 1,\n}\n\
         println(Lines.green, Commas.blue, Mixed.blue)\n\
         println(Point {\n  x: 1\n}, Pair { left: 1, right: 2 }, S { name: \"s\" })\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        stdout,
        "Lines.green Commas.blue Mixed.blue\n\
         struct {\"x\": 1, \"y\": 2} struct {\"left\": 1, \"right\": 2} struct {\"name\": \"s\", \"age\": 1}\n"
    );

    for (source, message) in [
        (
            "val Point = struct {\n    x: num y: num\n}\n",
            "expected comma or newline between struct fields",
        ),
        (
            "val Point = struct { x: num }\nprintln(Point { x: 1 y: 2 })\n",
            "expected comma between struct fields",
        ),
        (
            "val Point = struct { x, y }\nprintln(Point {\n  x: 1\n  y: 2\n})\n",
            "expected comma between struct fields",
        ),
        (
            "enum Colour { red green }\n",
            "expected comma or newline between enum cases",
        ),
        (
            "enum Colour { red; green }\n",
            "expected comma or newline between enum cases",
        ),
        (
            "val Point = struct { x; y }\n",
            "expected comma or newline between struct fields",
        ),
        ("enum Colour { red,, green }\n", "expected enum case name"),
        (
            "val Point = struct { x,, y }\n",
            "expected struct field name",
        ),
        (
            "val Point = struct { x, y }\nprintln(Point { x: 1,, y: 2 })\n",
            "expected struct field name",
        ),
    ] {
        let (code, _, stderr) = run(source);
        assert_eq!(code, Some(1), "{source}");
        assert!(
            stderr.contains(&format!("parse error: {message}")),
            "{source}: {stderr}"
        );
    }
    fs::remove_file(&path).expect("remove separator source");
}

#[test]
fn double_quoted_strings_escape_backspace_and_form_feed() {
    let path = fixture_path("backspace-form-feed");
    fs::write(
        &path,
        "println(\"\\b\" == \"\\10\", \"\\f\" == \"\\14\", len(\"a\\bb\\fc\"), '\\b' == \"\\\\b\")\n",
    )
    .expect("write escape source");
    let output = slug().arg(&path).output().expect("run escape source");
    fs::remove_file(&path).expect("remove escape source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "true true 5 true\n"
    );
}

#[test]
fn unconstrained_parameters_are_dynamic_inside_their_body() {
    let path = fixture_path("unconstrained-parameters");
    let run = |source: &str| {
        fs::write(&path, source).expect("write unconstrained parameter source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run unconstrained parameter source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    let (code, stdout, stderr) = run("val identity = fn(value) { value }\n\
         val text:str = identity(\"ok\")\n\
         val repeat = fn():str {\n\
         \x20 val go = fn(i, out) { if (i == 0) { out } else { recur(i - 1, out + \"x\") } }\n\
         \x20 go(3, \"\")\n\
         }\n\
         val collect = fn(i, out) { if (i == 0) { out } else { recur(i - 1, out :+ i) } }\n\
         val items:list = collect(2, [])\n\
         println(text, repeat(), items)\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "ok xxx [2, 1]\n");

    // An explicit any|nil annotation keeps its static meaning.
    let (code, _, stderr) =
        run("val pass = fn(value:any|nil) { value }\nval text:str = pass(\"a\")\n");
    fs::remove_file(&path).expect("remove unconstrained parameter source");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("semantic error: expected str, got any|nil"),
        "{stderr}"
    );
}

#[test]
fn compatible_parameter_requirements_combine_to_the_narrower_type() {
    let path = fixture_path("parameter-requirements");
    let run = |source: &str| {
        fs::write(&path, source).expect("write parameter requirement source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run parameter requirement source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    // `e` is a number from `e < 0` and is also passed on to an unconstrained
    // parameter, in either order.
    let (code, stdout, stderr) = run(
        "val pass = fn(flag, value) { if (flag) { value } else { 0 } }\n\
         val before = fn(e) { if (e < 0) { -e } else { pass(true, e) } }\n\
         val after = fn(e) { pass(true, e) * (if (e < 0) { -1 } else { 1 }) }\n\
         println(before(-3), before(2), after(-4))\n",
    );
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "3 2 4\n");

    // The combined requirement is still checked at calls.
    let (code, _, stderr) = run("val pass = fn(value) { value }\n\
         val f = fn(e) { if (e < 0) { 1 } else { pass(e) } }\n\
         f(\"x\")\n");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("semantic error"), "{stderr}");

    // Requirements with no common type still conflict.
    let (code, _, stderr) = run("val g = fn(x) { if (x < 0) { 1 } else { -x } }\n\
         val h = fn(x) { if (x < 0) { 1 } else { x < \"a\" } }\n");
    fs::remove_file(&path).expect("remove parameter requirement source");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("parameter `x` has incompatible body requirements: num and str"),
        "{stderr}"
    );
}

#[test]
fn match_bindings_exclude_types_matched_by_earlier_cases() {
    let path = fixture_path("match-binding-narrowing");
    let run = |source: &str| {
        fs::write(&path, source).expect("write match narrowing source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run match narrowing source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    // After `nil =>`, the binding, a wildcard's subject name, and a binding
    // after a constrained case are all `str`.
    let (code, stdout, stderr) = run("val g = fn(x):str|nil { x }\n\
         val f = fn(p):str { match g(p) { nil => \"none\" ; line => line } }\n\
         val h = fn(p):str { val v = g(p)\n match v { nil => \"none\" ; _ => v } }\n\
         val k = fn(p):nil { match g(p) { s: str => nil ; other => other } }\n\
         println(f(\"a\"), f(nil), h(\"b\"), k(nil))\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "a none b nil\n");

    // A guarded case may not match, so it narrows nothing, and the error
    // covers the expression that produced the result.
    let (code, _, stderr) = run("val g = fn(x):str|nil { x }\n\
         val f = fn(p, q):str {\n  match g(p) { nil if q => \"\" ; line => line }\n}\n");
    fs::remove_file(&path).expect("remove match narrowing source");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("expected str, found str|nil"), "{stderr}");
    assert!(stderr.contains(":3:3\n"), "{stderr}");
}

#[test]
fn recursive_function_variables_accept_reassignment() {
    let path = fixture_path("recursive-function-variable");
    let run = |source: &str| {
        fs::write(&path, source).expect("write recursive function variable source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run recursive function variable source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    // The recursive function's result is not inferred, so any function with
    // the same parameters may replace it, at module level or locally.
    let (code, stdout, stderr) = run(
        "var f = fn(n) { if (n == 0) { \"old\" } else { f(n - 1) } }\n\
         val first = f(1)\n\
         f = fn(n) { \"new\" }\n\
         val local = fn() {\n\
           var g = fn(n) { if (n == 0) { \"old\" } else { g(n - 1) } }\n\
           g = fn(n) { \"new\" }\n\
           g(0)\n\
         }\n\
         println(first, f(0), local())\n",
    );
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "old new new\n");

    // Known results must still match.
    let (code, _, stderr) = run("var g = fn(n:num):num { n }\ng = fn(n:num):str { \"x\" }\n");
    fs::remove_file(&path).expect("remove recursive function variable source");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("expected fn<num, num>, found fn<str, num>"),
        "{stderr}"
    );
}

#[test]
fn early_returns_join_and_check_a_functions_result() {
    let path = fixture_path("early-return-results");
    let run = |source: &str| {
        fs::write(&path, source).expect("write early return source");
        let output = slug().arg(&path).output().expect("run early return source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    // A recursive helper that returns early has its returns' type, not
    // `never`, and a return inside a match case or an onerror handler counts.
    let (code, stdout, stderr) = run("val peel = fn(n, out) {\n\
           if (n == 0) { return out }\n\
           recur(n - 1, out :+ n)\n\
         }\n\
         val pick = fn(x):str { match x { 0 => { return \"zero\" }\n _ => \"other\" } }\n\
         val rescue = fn():str {\n\
           defer onerror(e) { return e.msg }\n\
           throw Error { type: \"t\", msg: \"caught\" }\n\
         }\n\
         println(peel(3, [])[0], pick(0), rescue())\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "3 zero caught\n");

    // An early return's type is part of the inferred result.
    let (code, _, stderr) =
        run("val f = fn(n) {\n  if (n == 0) { return \"a\" }\n  5\n}\nval x:num = f(0)\n");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("expected num, found num|str"), "{stderr}");

    // A declared result is checked at each return.
    let (code, _, stderr) = run("val f = fn(n):num {\n  if (n == 0) { return \"zero\" }\n  n\n}\n");
    fs::remove_file(&path).expect("remove early return source");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("expected num, found str"), "{stderr}");
    assert!(stderr.contains(":2:24\n"), "{stderr}");
}

#[test]
fn a_nil_first_value_takes_its_var_type_from_the_first_assignment() {
    let path = fixture_path("nil-first-value");
    let run = |source: &str| {
        fs::write(&path, source).expect("write nil first value source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run nil first value source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    // A `var` that starts as `nil` takes `nil` with the type of its first
    // assignment, even one in a closure, and an accumulator's is found from
    // its other assignments; an annotated `var` keeps its annotation; a
    // field that starts as `nil` may hold anything; and a `val` stays
    // exactly `nil`.
    let (code, stdout, stderr) = run("var best = nil\n\
         best = 5\n\
         val first:num = best\n\
         best = nil\n\
         var held = nil\n\
         val keep = fn(v:num) { held = v }\n\
         keep(2)\n\
         val doubled = if (held != nil) { held * 2 } else { 0 }\n\
         var count = nil\n\
         val bump = fn() { if (count == nil) { count = 0 } else { count = count + 1 } }\n\
         bump()\n\
         bump()\n\
         var mixed:num|str|nil = nil\n\
         mixed = 5\n\
         mixed = \"s\"\n\
         val none = nil\n\
         val kept:str|nil = none\n\
         val S = struct { e = nil, n = 0 }\n\
         println(first, best, doubled, count, mixed, kept, S { e: \"text\" }.e, S { e: {k: 1} }.e)\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "5 nil 4 1 s nil text {\"k\": 1}\n");

    // A later assignment of another type is a mismatch where it is written,
    // and a mismatch at a use names the first assignment. The first
    // assignment is the first written, even in a closure that runs later.
    for (source, message) in [
        (
            "var best = nil\nbest = 5\nbest = \"s\"\n",
            "expected num|nil, got str: `best` is `num|nil`, from its first value `nil` and \
             its first assignment at 2:8; to hold both, declare `var best:num|str|nil = nil`",
        ),
        (
            "var best = nil\nbest = 5\nval s:str = best\n",
            "expected str, got num: `best` is `num|nil`, from its first value `nil` and its \
             first assignment at 2:8",
        ),
        (
            "var x = nil\nval later = fn() { x = \"late\" }\nx = 5\n",
            "expected str|nil, got num: `x` is `str|nil`, from its first value `nil` and its \
             first assignment at 2:24",
        ),
    ] {
        let (code, _, stderr) = run(source);
        assert_eq!(code, Some(1), "{source}");
        assert!(stderr.contains(message), "{source}: {stderr}");
    }

    // A concrete first value still fixes the type.
    for source in [
        "var x = 0\nx = \"s\"\n",
        "var x = 0\nx = nil\n",
        "val S = struct { n = 0 }\nS { n: \"seven\" }\n",
    ] {
        let (code, _, stderr) = run(source);
        assert_eq!(code, Some(1), "{source}");
        assert!(
            stderr.contains("semantic error: expected num"),
            "{source}: {stderr}"
        );
    }

    // With no assignment its type is `any|nil`, so a mismatch names the
    // declaration to annotate.
    for source in [
        "var n = 1\nvar held = nil\nval take = fn() { if (held != nil) { n = held } }\n",
        "var held = nil\nval take = fn():num { if (held != nil) { return held }; 0 }\n",
        "val P = struct { n:num }\nvar held = nil\nval make = fn() { if (held != nil) { P { n: held } } }\n",
    ] {
        let (code, _, stderr) = run(source);
        assert_eq!(code, Some(1), "{source}");
        assert!(
            stderr.contains(
                "expected num, got any: `held` starts as `nil`, so its type is `any|nil`; \
                 declare the type it holds, as in `var held:num|nil = nil`"
            ),
            "{source}: {stderr}"
        );
    }
    let (code, _, stderr) = run("var held:num|nil = nil\nvar n = 1\n\
         val take = fn() { if (held != nil) { n = held } }\nheld = 2\ntake()\nprintln(n)\n");
    assert_eq!(code, Some(0), "{stderr}");
    fs::remove_file(&path).expect("remove nil first value source");
}

#[test]
fn a_binding_keeps_the_dynamic_type_of_its_value() {
    let path = fixture_path("dynamic-binding");
    fs::write(
        &path,
        "val helper = fn(out) { out }\n\
         val block = fn() { val b = helper([1])\nb }\n\
         var other = []\n\
         other = if (len(other) > 0) { [2] } else { block() }\n\
         val [first, second] = helper([3, 4])\n\
         val nested = [helper(5)]\n\
         val five:num = nested[0]\n\
         println(other, first + second, five)\n",
    )
    .expect("write dynamic binding source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run dynamic binding source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "[1] 7 5\n");

    // An annotation still sets the binding's type.
    fs::write(&path, "val b:any|nil = 1\nval n:num = b\n").expect("write annotated source");
    let output = slug().arg(&path).output().expect("run annotated source");
    fs::remove_file(&path).expect("remove dynamic binding source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("expected num, got any|nil"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_nested_function_narrows_what_the_body_requires_of_a_parameter() {
    let path = fixture_path("nested-parameter-requirements");
    let run = |source: &str| {
        fs::write(&path, source).expect("write nested requirement source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run nested requirement source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    // The body requires `num|str` of `x`; a nested function narrows that to
    // `num`, in either order.
    let (code, stdout, stderr) = run("val show = fn(value:num|str):str { \"$value\" }\n\
         val before = fn(x) { show(x); val negate = fn() { -x }; negate() }\n\
         val after = fn(x) { val negate = fn() { -x }; show(x); negate() }\n\
         println(before(4), after(5))\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "-4 -5\n");

    // The narrowed requirement is checked at calls.
    let (code, _, stderr) = run("val show = fn(value:num|str):str { \"$value\" }\n\
         val f = fn(x) { show(x); val negate = fn() { -x }; negate() }\n\
         f(\"a\")\n");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("expected num, got str"), "{stderr}");

    // A nested function adds no requirement of its own, and its own
    // parameter of the same name is a different binding.
    let (code, stdout, stderr) = run("val show = fn(value:num|str):str { \"$value\" }\n\
         val lazy = fn(x) { val negate = fn() { -x }; x }\n\
         val shadow = fn(x) { show(x); val negate = fn(x) { -x }; negate(3) }\n\
         println(lazy(\"a\"), shadow(\"b\"))\n");
    fs::remove_file(&path).expect("remove nested requirement source");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "a -3\n");
}

#[test]
fn a_match_case_that_narrows_nothing_keeps_the_subject_type_it_reached() {
    let path = fixture_path("match-survivor-types");
    // The second case narrows nothing, so after the first match `body` may
    // still be anything but a string, and the second match's wildcard is
    // reachable. A case that returns still narrows what follows.
    fs::write(
        &path,
        "val kind = fn(body) {\n\
           val size = match body { text:str => 1; other => 2 }\n\
           match body { _:str => \"text\"; _ => \"other\" }\n\
         }\n\
         val next = fn(x:num|nil) {\n\
           match x { nil => return 0; _ => nil }\n\
           x + 1\n\
         }\n\
         println(kind(\"s\"), kind(0x\"00\"), next(2), next(nil))\n",
    )
    .expect("write match survivor source");
    let output = slug()
        .arg(&path)
        .output()
        .expect("run match survivor source");
    fs::remove_file(&path).expect("remove match survivor source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "text other 3 0\n"
    );
}

#[test]
fn list_literals_of_differing_types_are_tuples() {
    let path = fixture_path("tuple-literals");
    let run = |source: &str| {
        fs::write(&path, source).expect("write tuple source");
        let output = slug().arg(&path).output().expect("run tuple source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    // A pair of a list and a map keeps each element's type, through a
    // constant index, a negative one, and destructuring, and is still a list.
    let (code, stdout, stderr) = run(
        "val parse = fn() { [[1, 2], {host: \"x\", te: \"chunked\"}] }\n\
         val parsed = parse()\n\
         val [parts, headers] = parse()\n\
         println(parsed[1][\"te\"], parsed[-1].host, len(parsed[0]), headers.host, parts[0] + 1)\n\
         println(len(parsed), [...parsed, 3], parsed :+ 4, {[[1, \"a\"]]: \"key\"}[[1, \"a\"]])\n\
         var grow = [1, \"a\"]\n\
         grow = grow :+ 2\n\
         val pairs = [[1, \"a\"]] :+ [2, \"b\"]\n\
         println(grow, pairs[1][1] + \"!\", [1, 2, 3] :+ 4)\n",
    );
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        stdout,
        "chunked x 2 x 2\n\
         2 [[1, 2], {\"host\": \"x\", \"te\": \"chunked\"}, 3] [[1, 2], {\"host\": \"x\", \"te\": \"chunked\"}, 4] key\n\
         [1, \"a\", 2] b! [1, 2, 3, 4]\n"
    );

    // A list literal checked against a tuple annotation has its elements'
    // types, in a declaration, a function result, and a return.
    let (code, stdout, stderr) = run("val point:[num, num] = [1, 2]\n\
         val bounds = fn(a:num, b:num):[num, num] { if (a > b) { return [b, a] }\n [a, b] }\n\
         val split = fn():[list, map] { [[1], {a: 1}] }\n\
         println(point, bounds(3, 1)[1] + 1, split()[1][\"a\"])\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "[1, 2] 4 1\n");

    for (source, message) in [
        (
            "val p:[num, str] = [1, 2]\n",
            "expected [num, str], got [num, num]",
        ),
        (
            "val p:[num, num] = [1, 2, 3]\n",
            "expected [num, num], got list<num>",
        ),
    ] {
        let (code, _, stderr) = run(source);
        assert_eq!(code, Some(1), "{source}");
        assert!(stderr.contains(message), "{source}: {stderr}");
    }
    fs::remove_file(&path).expect("remove tuple source");
}

#[test]
fn nil_tests_combined_with_and_or_and_not_narrow_each_binding() {
    let path = fixture_path("combined-nil-tests");
    fs::write(
        &path,
        "val both = fn(a:num|nil, b:num|nil):num {\n\
             if (a != nil && b != nil) { a + b } else { 0 }\n\
         }\n\
         val either = fn(a:num|nil, b:num|nil):num {\n\
             if (a == nil || b == nil) { return 0 }\n\
             a + b\n\
         }\n\
         val negated = fn(a:num|nil):num { if (!(a == nil)) { a } else { 1 } }\n\
         val member = fn(r:map):num { if (r.x != nil && r.y != nil) { r.x + r.y } else { 0 } }\n\
         println(both(1, 2), both(nil, 2), either(3, 4), either(3, nil), negated(5), negated(nil), member({x: 1, y: 2}))\n",
    )
    .expect("write combined nil test source");
    let output = slug().arg(&path).output().expect("run combined nil tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "3 0 7 0 5 1 3\n");

    // Either side of a holding `||` may be nil.
    fs::write(
        &path,
        "val f = fn(a:num|nil, b:num|nil):num { if (a != nil || b != nil) { a + b } else { 0 } }\n",
    )
    .expect("write disjunction source");
    let output = slug().arg(&path).output().expect("run disjunction source");
    fs::remove_file(&path).expect("remove combined nil test source");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("operator `+` does not accept num|nil"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_nil_test_narrows_a_member_of_an_immutable_binding() {
    let path = fixture_path("member-narrowing");
    let run = |source: &str| {
        fs::write(&path, source).expect("write member narrowing source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run member narrowing source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let prelude = "val f = fn(n) { if (n > 0) { {error: {at: 1}} } else { {error: nil} } }\n";

    // A member tested against nil, by `.` or `[]`, through `&&`, past an early
    // return, and inside a function defined on the narrowed path.
    let (code, stdout, stderr) = run(&format!(
        "{prelude}\
         val r = f(1)\n\
         if (r.error != nil) {{ println(r.error + {{kind: \"dot\"}}) }}\n\
         if (r[\"error\"] != nil) {{ println(r[\"error\"] + {{kind: \"index\"}}) }}\n\
         val check = fn(result) {{\n\
             if (result.error == nil) {{ return \"ok\" }}\n\
             result.error + {{kind: \"early\"}}\n\
         }}\n\
         println(check(r), check(f(0)))\n\
         val both = fn(a) {{ if (a.x != nil && a.x > 1) {{ \"big\" }} else {{ \"small\" }} }}\n\
         println(both({{x: 2}}), both({{x: nil}}))\n\
         if (r.error != nil) {{ val later = fn() {{ r.error + {{kind: \"closure\"}} }}\nprintln(later()) }}\n"
    ));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        stdout,
        "{\"at\": 1, \"kind\": \"dot\"}\n\
         {\"at\": 1, \"kind\": \"index\"}\n\
         {\"at\": 1, \"kind\": \"early\"} ok\n\
         big small\n\
         {\"at\": 1, \"kind\": \"closure\"}\n"
    );

    // A `var`'s member, and a member after branches that disagree, stay
    // possibly nil.
    for source in [
        "var v = f(1)\nif (v.error != nil) { v.error + {kind: \"x\"} }\n",
        "val r = f(1)\nif (r.error != nil) { 1 } else { 2 }\nr.error + {kind: \"x\"}\n",
    ] {
        let (code, _, stderr) = run(&format!("{prelude}{source}"));
        assert_eq!(code, Some(1), "{source}");
        assert!(
            // A `var` keeps its map literal's map type; a `val` its record
            // type.
            stderr.contains("operator `+` does not accept map<str, num>|nil and {kind: str}")
                || stderr.contains("operator `+` does not accept {at: num}|nil and {kind: str}"),
            "{source}: {stderr}"
        );
    }
    fs::remove_file(&path).expect("remove member narrowing source");
}

#[test]
fn branches_rejoin_with_the_union_of_their_types() {
    let path = fixture_path("branch-merge");
    let run = |source: &str| {
        fs::write(&path, source).expect("write branch merge source");
        let output = slug().arg(&path).output().expect("run branch merge source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    // A branch that assigns `nil` to a narrowed `var` makes it possibly nil
    // again after the branches rejoin, where it previously kept the narrowed
    // type and failed only when that branch ran.
    let (code, _, stderr) = run("val f = fn(c) {\n\
           var x:num|nil = 5\n\
           if (x != nil) {\n\
             if (c) { x = nil }\n\
             x + 1\n\
           }\n\
         }\n");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("operator `+` does not accept num|nil and num"),
        "{stderr}"
    );

    // Branches that agree keep their type, and narrowing in both keeps it.
    let (code, stdout, stderr) = run("val f = fn(c) {\n\
           var x:num|nil = nil\n\
           if (c) { x = 1 } else { x = 2 }\n\
           val y:num = x\n\
           y + 1\n\
         }\n\
         val g = fn(v:num|nil) {\n\
           if (v == nil) { return 0 }\n\
           if (v > 1) { v } else { v + 1 }\n\
         }\n\
         println(f(true), f(false), g(nil), g(5), g(1))\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "2 3 0 5 2\n");
    fs::remove_file(&path).expect("remove branch merge source");
}

#[test]
fn lists_maps_and_tasks_are_covariant_and_channels_are_not() {
    let path = fixture_path("collection-variance");
    let run = |source: &str| {
        fs::write(&path, source).expect("write collection variance source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run collection variance source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    let (code, stdout, stderr) = run("val mixed:list<num|str> = [1, 2]\n\
         var record:map<str, any|nil> = {kind: \"text\"}\n\
         record = {kind: \"number\", value: 1}\n\
         val nested:list<list<num|nil>> = [[1], [2]]\n\
         val widen = fn(pending:task<num>) { val wider:task<num|str> = pending; wider }\n\
         println(mixed, record, nested)\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        stdout,
        "[1, 2] {\"kind\": \"number\", \"value\": 1} [[1], [2]]\n"
    );

    // A covered union member is dropped, so a mismatch names one map type.
    for (source, message) in [
        (
            "val numbers:list<num> = [1, \"a\"]\n",
            "expected list<num>, got list<num|str>",
        ),
        (
            "val c:chan<num|str> = chan<num>(1)\n",
            "expected chan<num|str>, got chan<num>",
        ),
        (
            "val pick = fn(c) { if (c) { {a: 1} } else { {a: 1, b: nil} } }\nval n:num = pick(true)\n",
            "expected num, got map<str, num|nil>",
        ),
    ] {
        let (code, _, stderr) = run(source);
        assert_eq!(code, Some(1), "{source}");
        assert!(stderr.contains(message), "{source}: {stderr}");
    }

    // The more specific overload still wins, whichever is declared first.
    let (code, stdout, stderr) = run("val f = fn(xs:list<num|str>) { \"mixed\" }\n\
         val f = fn(xs:list<num>) { \"numbers\" }\n\
         println(f([1]), f([1, \"a\"]))\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "numbers mixed\n");
    fs::remove_file(&path).expect("remove collection variance source");
}

#[test]
fn indexing_a_union_of_collections_yields_any_member_result() {
    let path = fixture_path("union-index");
    let run = |source: &str| {
        fs::write(&path, source).expect("write union index source");
        let output = slug().arg(&path).output().expect("run union index source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    let (code, stdout, stderr) = run(
        "val pick = fn(c) { if (c) { {a: 1} } else { {a: \"x\", b: nil} } }\n\
         val list:list<num>|list<str> = [1, 2]\n\
         val text:str|list<str> = \"ab\"\n\
         println(pick(true).a, pick(false)[\"b\"], list[0], list[1:], text[0], text[1:])\n",
    );
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "1 nil 1 [2] a b\n");

    for (source, message) in [
        (
            "val pick = fn(c) { if (c) { {a: 1} } else { {a: \"x\"} } }\nval n:num = pick(true).a\n",
            "expected num, got num|str|nil",
        ),
        (
            "val m:map<str, num>|nil = nil\nm.a\n",
            "operator `[]` does not accept map<str, num>|nil and str",
        ),
        (
            "val l:list<num>|list<str> = [1]\nl[\"a\"]\n",
            "operator `[]` does not accept list<num>|list<str> and str",
        ),
    ] {
        let (code, _, stderr) = run(source);
        assert_eq!(code, Some(1), "{source}");
        assert!(stderr.contains(message), "{source}: {stderr}");
    }
    fs::remove_file(&path).expect("remove union index source");
}

#[test]
fn nil_coalescing_removes_nil_from_the_left_type() {
    let path = fixture_path("nil-coalescing-types");
    let run = |source: &str| {
        fs::write(&path, source).expect("write nil coalescing type source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run nil coalescing type source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let (code, stderr) = run("val maybe = fn(x):str|nil { x }\nval s:str = maybe(nil) ?? \"\"\n");
    assert_eq!(code, Some(0), "{stderr}");
    let (code, stderr) = run("val maybe = fn(x):str|nil { x }\nval n:num = maybe(nil) ?? 0\n");
    fs::remove_file(&path).expect("remove nil coalescing type source");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("expected num, found num|str"), "{stderr}");
}

#[test]
fn an_error_handlers_value_joins_the_function_result() {
    let path = fixture_path("onerror-result");
    let run = |source: &str| {
        fs::write(&path, source).expect("write onerror result source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run onerror result source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let (code, stdout, stderr) = run("val errorOf = fn(f) {\n\
           defer onerror(e) { e }\n\
           f()\n\
           nil\n\
         }\n\
         val caught = errorOf(fn() { throw Error { type: \"t\", msg: \"m\" } })\n\
         println(caught.type, errorOf(fn() { 1 }))\n");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "t nil\n");

    // A declared result is checked against the handler's value.
    let (code, _, stderr) = run("val f = fn():str {\n  defer onerror(e) { 0 }\n  \"ok\"\n}\n");
    fs::remove_file(&path).expect("remove onerror result source");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("expected str, found num"), "{stderr}");
}

#[test]
fn std_list_helpers_accept_lists_containing_nil() {
    let path = fixture_path("std-lists-with-nil");
    fs::write(
        &path,
        "val {map, find} = import(\"slug.std\")\n\
         val flags = map([0, nil, \"2\"], fn(v) { v == nil })\n\
         val lengths = map([\"a\", \"bb\"], fn(s) { len(s) })\n\
         val found:num|nil = find([nil, 3, 4], fn(v) { v != nil && v > 3 })\n\
         println(flags, lengths, found)\n",
    )
    .expect("write std nil list source");
    let output = slug().arg(&path).output().expect("run std nil list source");
    fs::remove_file(path).expect("remove std nil list source");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[false, true, false] [1, 2] 4\n"
    );
}

#[test]
fn a_declaration_replaces_an_implicit_builtin_of_the_same_name() {
    let path = fixture_path("builtin-shadowing");
    let run = |source: &str| {
        fs::write(&path, source).expect("write builtin shadowing source");
        let output = slug()
            .arg(&path)
            .output()
            .expect("run builtin shadowing source");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    // `fs.close` is one function, so an unannotated or nil-initialized
    // argument resolves to it, and the builtin `close` still closes channels.
    let (code, stdout, stderr) = run("val fs = import(\"slug.io.fs\")\n\
         val closeFile = fn(file) { fs.close(file) }\n\
         var file = nil\n\
         file = fs.openWrite(\"builtin-shadowing.txt\")\n\
         if (file != nil) { closeFile(file) }\n\
         val ch = chan(1)\n\
         close(ch)\n\
         println(\"closed\")\n");
    let _ = fs::remove_file(path.with_file_name("builtin-shadowing.txt"));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout, "closed\n");

    // A module's own `close` replaces the builtin there, as it does at runtime.
    let (code, _, stderr) = run("val close = fn(x:num) { x }\nclose(chan(1))\n");
    fs::remove_file(&path).expect("remove builtin shadowing source");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("semantic error: expected num"), "{stderr}");
}

#[test]
fn a_type_parameter_reaches_a_parameter_of_any_value() {
    let path = fixture_path("generic-to-top");
    fs::write(
        &path,
        "val show = fn(x:any|nil) { x }\n\
         val loose = fn(x) { x }\n\
         val pass = fn<T>(x:T):T { show(x)\nloose(x)\nx }\n\
         println(pass(1), pass(nil))\n",
    )
    .expect("write generic to top");
    let output = slug().arg(&path).output().expect("run generic to top");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "1 nil\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_value_the_checker_could_not_prove_is_checked_where_it_reaches_an_annotation() {
    let prelude = "val dynamic = fn(x) { x }\n";
    for (source, message) in [
        (
            "val port:num = dynamic(\"80\")",
            "expected num, got str: the value of `port`",
        ),
        (
            "var n:num = 0\nn = dynamic(\"s\")",
            "expected num, got str: the value assigned to `n`",
        ),
        (
            "val inc = fn(x:num):num { x + 1 }\ninc(dynamic(\"s\"))",
            "expected num, got str: the argument `x`",
        ),
        (
            "val tenth = fn(v) { v / 10 }\ntenth(dynamic(\"s\"))",
            "expected num, got str: the argument `v`, whose type the function's body relies on",
        ),
        (
            "val name = fn(x):str { dynamic(x) }\nname(3)",
            "expected str, got num: the function's result",
        ),
        (
            "val name = fn(x):str { if (x == 0) { return dynamic(x) }\n\"s\" }\nname(0)",
            "expected str, got num: the function's result",
        ),
        (
            "val Pet = struct { name: str }\nPet { name: dynamic(3) }",
            "expected str, got num: the field `name`",
        ),
        (
            "val total = fn(xs:list<num>):num { len(xs) }\ntotal(dynamic([1, \"a\"]))",
            "expected list<num>, got list: the argument `xs`",
        ),
        (
            "val launder = fn<T>(x:T):T { dynamic(x) }\nval n:num = launder(1)\nval s:str = launder(\"s\")\nprintln(s)",
            "",
        ),
        // A recovering handler's value is the function's result.
        (
            "val safe = fn(x):num {\ndefer onerror(e) { dynamic(x) }\nthrow \"boom\"\n}\nsafe(\"s\")",
            "expected num, got str: the function's result",
        ),
        // The schema checks its fields where it is not known.
        (
            "val Pet = struct { name: str }\nval make = fn(s, v) { s { name: v } }\nmake(Pet, 7)",
            "expected str, got num: the field `name`",
        ),
        (
            "val Pet = struct { name: str }\nval rename = fn(p, v) { p copy { name: v } }\nrename(Pet { name: \"a\" }, 7)",
            "expected str, got num: the field `name`",
        ),
        // An overload set called as a value takes the member that accepts.
        (
            "val f = fn(x:num) { \"num\" }\nval f = fn(x:str) { \"str\" }\nval apply = fn(h, v) { h(v) }\nif (apply(f, \"a\") != \"str\") { throw \"wrong member\" }\napply(f, true)",
            "expected num, got bool: the argument `x`",
        ),
    ] {
        let path = fixture_path("annotation-check");
        fs::write(&path, format!("{prelude}{source}\n")).expect("write annotation check");
        let output = slug().arg(&path).output().expect("run annotation check");
        fs::remove_file(&path).expect("remove annotation check");
        let stderr = String::from_utf8_lossy(&output.stderr);
        if message.is_empty() {
            assert!(output.status.success(), "{source}: {stderr}");
        } else {
            assert_eq!(output.status.code(), Some(1), "{source}: {stderr}");
            assert!(stderr.contains(message), "{source}: {stderr}");
        }
    }
}

#[test]
fn a_proven_value_and_a_conforming_one_pass_their_checks() {
    let path = fixture_path("passing-annotation-checks");
    fs::write(
        &path,
        "val dynamic = fn(x) { x }\n\
         val inc = fn(x:num):num { x + 1 }\n\
         val {map} = import(\"slug.std\")\n\
         val Pet = struct { name: str }\n\
         val r:{name: str} = dynamic({name: \"a\", age: 2})\n\
         println(inc(1), inc(dynamic(2)), map([\"a\", \"bb\"], fn(s) { len(s) }), Pet { name: dynamic(\"Rex\") }.name, r.name)\n",
    )
    .expect("write passing checks");
    let output = slug().arg(&path).output().expect("run passing checks");
    fs::remove_file(&path).expect("remove passing checks");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "2 3 [1, 2] Rex a\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn tuples_recursive_types_and_generic_structs_are_checked_deeply() {
    let path = fixture_path("deep-annotation-checks");
    fs::write(
        &path,
        "val {Result, or} = import(\"slug.std\")\n\
         val dynamic = fn(x) { x }\n\
         type Tree = {value: num, children: list<Tree>}\n\
         val Box = struct<T> { value: T }\n\
         val second = fn(p:[num, str]):str { p[1] }\n\
         val root = fn(t:Tree):num { t.value }\n\
         val unbox = fn(b:struct<Box<num>>):num { b.value }\n\
         val take = fn(r:Result<num, str>):num { r /> or(0) }\n\
         enum Why {\n  Gone\n  Bad { code: num }\n}\n\
         val why = fn(r:Result<num, Why>):num { r /> or(-1) }\n\
         val tries = [\n\
           fn() { second(dynamic([\"a\", 1])) },\n\
           fn() { root(dynamic({value: 1, children: [{value: \"x\", children: []}]})) },\n\
           fn() { unbox(dynamic(Box { value: \"s\" })) },\n\
           fn() { take(dynamic(Result.Ok { value: \"no\" })) },\n\
         ]\n\
         val failed = fn(attempt) {\n\
           defer onerror(e) { return true }\n\
           attempt()\n\
           false\n\
         }\n\
         println(second(dynamic([1, \"a\"])), root(dynamic({value: 1, children: [{value: 2, children: []}]})), \
           unbox(dynamic(Box { value: 2 })), take(dynamic(Result.Ok { value: 3 })), \
           why(dynamic(Result.Error { error: Why.Bad { code: 1 } })), why(dynamic(Result.Error { error: Why.Gone })))\n\
         println(failed(tries[0]), failed(tries[1]), failed(tries[2]), failed(tries[3]))\n",
    )
    .expect("write deep checks");
    let artifact = path.with_extension("cslug");
    let source = slug().arg(&path).output().expect("run deep checks");
    let compile = slug()
        .arg("compile")
        .arg(&path)
        .arg(&artifact)
        .output()
        .expect("compile deep checks");
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let compiled = slug()
        .arg("run")
        .arg(&artifact)
        .output()
        .expect("run compiled deep checks");
    fs::remove_file(&path).expect("remove deep checks");
    fs::remove_file(&artifact).expect("remove compiled deep checks");
    for output in [source, compiled] {
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "a 1 2 3 -1 -1\ntrue true true true\n",
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn a_maps_own_keys_read_its_values() {
    let run = |source: &str| {
        let path = fixture_path("map-keys");
        fs::write(&path, source).expect("write map keys");
        let output = slug().arg(&path).output().expect("run map keys");
        fs::remove_file(&path).expect("remove map keys");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    // Over `keys(m)`, directly, through a `val`, or of a field's map, `m[k]`
    // is one of `m`'s values, not `nil`.
    let (stdout, stderr) = run("val {filter, reduce} = import(\"slug.std\")\n\
         val counts:map<str, num> = {a: 1, b: 2}\n\
         val total:num = reduce(keys(counts), 0, fn(sum, k) { sum + counts[k] })\n\
         val ks = keys(counts)\n\
         val go = fn(i:num, acc:num):num { if (i == len(ks)) { acc } else { recur(i + 1, acc + counts[ks[i]]) } }\n\
         type E = {holds: map<str, num>}\n\
         val held = fn(e:E):list<str> { filter(keys(e.holds), fn(k) { e.holds[k] > 1 }) }\n\
         println(total, go(0, 0), held({holds: {x: 1, y: 2}}))\n");
    assert_eq!(stdout, "3 3 [\"y\"]\n", "{stderr}");
    // Another map's keys are not this one's.
    let (_, stderr) = run("val {reduce} = import(\"slug.std\")\n\
         val a:map<str, num> = {x: 1}\n\
         val b:map<str, num> = {y: 2}\n\
         val total:num = reduce(keys(a), 0, fn(sum, k) { sum + b[k] })\n");
    assert!(stderr.contains("num|nil"), "{stderr}");
}

#[test]
fn an_alias_names_an_imported_type_and_a_checked_var_stays_proven() {
    // An alias may name an imported module's type; a `var` assigned an
    // unproven value is checked there, and of its type after.
    let path = fixture_path("alias-import");
    fs::write(
        &path,
        "val test = import(\"slug.test\")\n\
         type Outcome = {failure: test.Failure|nil, n: num}\n\
         var current:Outcome = {failure: nil, n: 0}\n\
         val dyn = fn(x) { x }\n\
         val bump = fn() {\n\
           current = dyn({failure: nil, n: current.n + 1})\n\
           current = current copy {n: current.n + 1}\n\
         }\n\
         bump()\n\
         println(current.n)\n",
    )
    .expect("write alias import");
    let output = slug().arg(&path).output().expect("run alias import");
    fs::remove_file(&path).expect("remove alias import");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "2\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_record_merge_sets_listed_fields_of_a_map_or_a_struct() {
    let run = |source: &str| {
        let path = fixture_path("record-merge");
        fs::write(&path, source).expect("write record merge");
        let output = slug().arg(&path).output().expect("run record merge");
        fs::remove_file(&path).expect("remove record merge");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    // A map and a struct of the record type are each updated, keeping it.
    let (stdout, stderr) = run("type E = {n: num, name: str}\n\
         val bump = fn(e:E):E { e + {n: e.n + 1} }\n\
         val P = struct { n: num, name: str, extra: bool }\n\
         println(bump({n: 1, name: \"map\"}), bump(P { n: 1, name: \"struct\", extra: true }))\n");
    assert_eq!(
        stdout,
        "{\"n\": 2, \"name\": \"map\"} struct {\"n\": 2, \"name\": \"struct\", \"extra\": true}\n",
        "{stderr}"
    );
    // A key the type does not list could be a field a struct lacks.
    let (_, stderr) = run("type E = {n: num}\nval f = fn(e:E) { e + {other: 1} }\n");
    assert!(
        stderr.contains(
            "which has no field `other`; a record merge sets only the fields its type lists"
        ),
        "{stderr}"
    );
    // A struct's merge is checked against its schema, as `copy` is.
    let (_, stderr) = run(
        "val P = struct { n: num }\nval dyn = fn(x) { x }\nprintln(dyn(P { n: 1 }) + {n: \"s\"})\n",
    );
    assert!(
        stderr.contains("expected num, got str: the field `n`"),
        "{stderr}"
    );
}

#[test]
fn a_call_checks_what_it_cannot_prove_and_enters_past_the_checks() {
    // `add`'s calls in `go` are proven and enter past its entry check, a
    // walk of `out`; the call in `bad` checks its unproven argument where it
    // is, and fails as the entry check would.
    let path = fixture_path("proven-calls");
    fs::write(
        &path,
        "val add = fn(out:list<str>, s:str):list<str> { out :+ s }\n\
         val go = fn(i:num, out:list<str>):list<str> {\n\
           if (i == 0) { out } else { go(i - 1, add(out, \"x\")) }\n\
         }\n\
         println(len(go(2000, [])))\n\
         val loose = fn(x) { x }\n\
         val bad = fn() { add(loose([1]), \"y\") }\n\
         bad()\n",
    )
    .expect("write proven calls");
    let output = slug().arg(&path).output().expect("run proven calls");
    fs::remove_file(&path).expect("remove proven calls");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "2000\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("expected list<str>, got list: the argument `out`"),
        "{stderr}"
    );
}

#[test]
fn a_nil_case_covers_nil() {
    let path = fixture_path("nil-covers-nil");
    fs::write(
        &path,
        "val f = fn(v:num|nil):num {\n\
           match v {\n\
             nil => 0\n\
             n:num => n\n\
           }\n\
         }\n\
         println(f(3), f(nil))\n",
    )
    .expect("write nil covers nil");
    let output = slug().arg(&path).output().expect("run nil covers nil");
    fs::remove_file(&path).expect("remove nil covers nil");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "3 0\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_call_that_only_throws_narrows_what_follows_it() {
    let path = fixture_path("never-narrows");
    fs::write(
        &path,
        "val fail = fn(why:str) { throw why }\n\
         val double = fn(v:num|nil):num {\n\
           if (v == nil) { fail(\"no value\") }\n\
           v * 2\n\
         }\n\
         println(double(4))\n",
    )
    .expect("write never narrows");
    let output = slug().arg(&path).output().expect("run never narrows");
    fs::remove_file(&path).expect("remove never narrows");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "8\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_list_literal_ending_a_branch_is_the_tuple_expected() {
    let path = fixture_path("tuple-branches");
    fs::write(
        &path,
        "val pair = fn(v:num|nil, e:num):[num, num] {\n\
           if (v == nil) { throw \"no value\" } else { [v, e] }\n\
         }\n\
         val pick = fn(k:str):[str, num] {\n\
           match k {\n\
             \"a\" => [\"first\", 1]\n\
             _ => [k, 0]\n\
           }\n\
         }\n\
         println(pair(1, 2), pick(\"a\"), pick(\"z\"))\n",
    )
    .expect("write tuple branches");
    let output = slug().arg(&path).output().expect("run tuple branches");
    fs::remove_file(&path).expect("remove tuple branches");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[1, 2] [\"first\", 1] [\"z\", 0]\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_type_from_a_module_that_does_not_compile_names_that_module() {
    // An import with no types here, as one that does not compile, makes a
    // type named through it say which module it is, not that the import is
    // unknown.
    let directory =
        std::env::temp_dir().join(format!("slug-cli-unavailable-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("make module directory");
    fs::write(
        directory.join("broken.slug"),
        "export type T = {a: num}\nexport val f = fn():num { \"x\" }\n",
    )
    .expect("write broken module");
    fs::write(
        directory.join("main.slug"),
        "val broken = import(\"broken\")\ntype U = {t: broken.T}\nprintln(1)\n",
    )
    .expect("write importer");
    let output = slug()
        .arg("main.slug")
        .current_dir(&directory)
        .output()
        .expect("run importer");
    fs::remove_dir_all(&directory).expect("remove module directory");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("`broken.T` cannot be resolved: module `broken`, imported as `broken`, has no types here"),
        "{stderr}"
    );
}

#[test]
fn a_named_function_given_to_a_parametric_function_is_checked_only_when_unproven() {
    let run = |start: &str| {
        let path = fixture_path("named-fold");
        fs::write(
            &path,
            format!(
                "val {{reduce}} = import(\"slug.std\")\n\
                 val step = fn(acc:list<num>, x:num):list<num> {{ acc :+ (x * 2) }}\n\
                 val dyn = fn(x) {{ x }}\n\
                 val start:list<num> = []\n\
                 println(reduce([1, 2, 3], {start}, step))\n"
            ),
        )
        .expect("write named fold");
        let output = slug().arg(&path).output().expect("run named fold");
        fs::remove_file(&path).expect("remove named fold");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    // A proven start: `step` is called past its checks, by name.
    let (stdout, stderr) = run("start");
    assert_eq!(stdout, "[2, 4, 6]\n", "{stderr}");
    // An unproven one is checked when `step` takes it.
    let (stdout, stderr) = run("dyn(\"no\")");
    assert_eq!(stdout, "", "{stderr}");
    assert!(stderr.contains("list<num>"), "{stderr}");
}
