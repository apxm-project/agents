# Skilled

A reference Agent Program that declares two Agent Skills and loads both.

A skill is instructions and supporting context. This package carries one as a
file — `skills/review/SKILL.md`, hashed into the package's integrity chain —
and writes the other in the program itself, where it is covered by the
artifact's source-bundle digest. Both are declared with the same `Skill`
marker, and both are loaded the same way: `await skill.load()` records an
ordinary `capability.invoke` on `read_skill`, so the artifact declares the
authority to read instructions exactly as it declares any other capability.

`tests/acceptance.py` compiles this package and executes it, playing the host:
it publishes the declared skills into a local discovery root and then runs the
compiled AIR through `apxm execute-canonical`, which is the same admitted path
every other capability takes.
