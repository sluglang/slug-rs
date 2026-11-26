# Testing a Program

Test a program's logic in Slug, in one process: put the logic in a module,
import that module from a test program, and run many assertions in one
`slug` run. Each test then costs a function call rather than a process, and
one run reports every failing test.

## Use the installed application runner

`lib/slug/test/runner.slug` supplies `slug.test.runner`. Tag ordinary
zero-argument test functions with `@test` and input/result examples with
`@testWith([arguments...], expected, ...)`. Export declarations that the test
entry should discover, then put this entry in `tests/all.slug`:

```slug
val runner = import("slug.test.runner")

val main = fn():num {
    runner.run(["app.logic", "checks"], fn(name:str):list { describe(name) })
}
```

Here `app.logic` names `src/app/logic.slug`, and `checks` names
`tests/checks.slug`. Define the discovery callback in the application entry
so relative module names resolve there rather than beside the installed
runner. Only exported declarations are described by default. No directory
scan or unexported-inspection permission is required.

Run with `"$SLUG_HOME/target/release/slug" tests/all.slug`. The runner executes
both tags, continues after failures, and prints separate pass, failure, and
unexpected-error counts. Each example counts as a test. Discovery and tag
errors are reported without counting as executed tests. Empty suites and
malformed tags fail. Duplicate module names run once. Passing `true` as the
third argument also prints passing tests.

Return the runner's 0/1 result from `main` so failures become a failing process
exit. Module initialization failures are caught when discovery occurs in the
callback, allowing other named modules to run. Tests execute in one process
and share module state; the runner does not reset that state between tests.

The separate installed `slug test` tool remains the runner for clutch suites.
The manual patterns below explain how assertions and custom runners work.

## Keep the logic in a module

The entry program handles only input and output: options, files, the
display. Everything a test wants to check lives in a module beside it, with
the state it works on passed in or kept there, and exported:

```slug
// robot.slug: the program's logic. main.slug, the entry program, imports it.
val { reduce } = import("slug.std")

export val newGame = fn():map { {x: 0, y: 0, moves: 0, log: []} }

// A game restored from a snapshot is just the snapshot's map.
export val load = fn(snapshot:map):map { snapshot }

export val command = fn(game:map, key:str):map {
    val step = match key {
        "l" => [1, 0]
        "h" => [-1, 0]
        "j" => [0, 1]
        "k" => [0, -1]
        _ => nil
    }
    if (step == nil) {
        game
    } else {
        val x = game.x + step[0]
        val y = game.y + step[1]
        if (x < 0 || x > 4 || y < 0 || y > 4) {
            game + {log: ["Bump!"]}
        } else {
            game + {x: x, y: y, moves: game.moves + 1, log: []}
        }
    }
}

export val commands = fn(game:map, keys:list):map {
    reduce(keys, game, fn(g, key) { command(g, key) })
}
```

## Write the tests as a program

`slug.test` (see [its source](../../clutch/slug.core.clutch/modules/test.slug)) has `assertEqual`,
`assertTrue`, `assertThrows`, and the rest; a failed assertion throws an
error holding the expected and actual values. A test program imports the
module, builds the states it needs (a new game, or a snapshot parsed from
JSON, as a saved game would be read), and runs each test in turn:

```slug
// tests.slug: run with `slug tests.slug`.
val test = import("slug.test")
val json = import("slug.fmt.json")
val robot = import("robot")

val corner = robot.load(json.parse("{\"x\": 4, \"y\": 4, \"moves\": 8, \"log\": []}"))

val tests = [
    ["a new game starts at the origin", fn() {
        val game = robot.newGame()
        test.assertEqual([game.x, game.y, game.moves], [0, 0, 0])
    }],
    ["moves change the position and count", fn() {
        val game = robot.commands(robot.newGame(), ["l", "l", "j"])
        test.assertEqual([game.x, game.y, game.moves], [2, 1, 3])
    }],
    ["walls stop the robot", fn() {
        val game = robot.command(corner, "l")
        test.assertEqual(game.x, 4)
        test.assertEqual(game.moves, 8, "a bump is not a move")
        test.assertEqual(game.log, ["Bump!"])
    }],
]

// Runs every test, even after one fails, and prints each failure with its
// message and values. A test that crashed, rather than failing an
// assertion, is run once more at the end without catching its error, so slug
// reports where it happened. Any failure makes the run exit with status 1.
val main = fn() {
    val run = fn(i:num, failed:num, crashed:num) {
        if (i == len(tests)) {
            [failed, crashed]
        } else {
            val name = tests[i][0]
            val failure = test.runTest(tests[i][1])
            if (failure == nil) {
                recur(i + 1, failed, crashed)
            } else {
                val msg = failure.msg
                println("FAIL $name: $msg")
                if (failure.error == nil) {
                    val expected = failure.expected
                    val actual = failure.actual
                    println("  expected: $expected")
                    println("  actual:   $actual")
                }
                val first = if (crashed < 0 && failure.error != nil) { i } else { crashed }
                recur(i + 1, failed + 1, first)
            }
        }
    }
    val [failed, crashed] = run(0, 0, -1)
    val count = len(tests)
    if (crashed >= 0) {
        val name = tests[crashed][0]
        println("running `$name` again for its full error:")
        tests[crashed][1]()
    }
    if (failed > 0) {
        test.fail("$failed of $count tests failed")
    }
    println("all $count tests passed")
}
```

Every failure is listed in one run, so you can fix several at once:

```
FAIL moves change the position and count: values differ
  expected: [2, 1, 4]
  actual:   [2, 1, 3]
FAIL walls stop the robot: a bump is not a move
  expected: 9
  actual:   8
slug: runtime error: uncaught throw: struct {"type": "test.failure", "msg": "2 of 3 tests failed", ...}
```

A test that crashes, here with an index past the end of a list, is listed
too, then run again, so slug reports the line that failed and the calls that
led to it, from the test down:

```
FAIL moves change the position and count: threw out_of_bounds: list index is out of bounds
FAIL walls stop the robot: threw out_of_bounds: list index is out of bounds
running `moves change the position and count` again for its full error:
slug: runtime error: list index is out of bounds
    --> robot.slug:20:30
  > 20 |         val x = game.x + step[2]
  ...
  in <fn #4> at tests.slug:14:25
```

Fix them and run `slug tests.slug` again; add a test for each rule as you
write it.

## Tests in a project

In a [project](configuration.md#project-root), the logic lives in `src/` and
the test programs in `tests/`. A test program imports the modules it tests
from `src/` by name, and helpers beside it in `tests/` by the usual
importer-relative rule:

```text
robot/
  slug.toml
  src/robot.slug        the logic: `import("robot")`
  src/main.slug         the entry program
  tests/all.slug        `slug tests/all.slug` imports robot and moves
  tests/moves.slug
```

Modules in `src/` cannot import tests, since `tests/` is outside the module
root. Run `slug tests/all.slug` from anywhere in the project.

## Checking the whole program

Test the entry program itself with a few end-to-end runs: run it as the
user would, then read what it wrote. When a script in another language runs
`slug` for you, have it print the program's stdout and stderr whenever the
run fails, since slug's diagnostic is on stderr; otherwise a failure shows
only an exit status.

Installed clutches can also tag functions with `@test` and `@testWith` and
run them with `slug test`; see `slug.test`'s source,
`clutch/slug.core.clutch/modules/test.slug`.
