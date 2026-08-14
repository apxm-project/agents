# Coder acceptance checks

`acceptance.py` verifies Coder's three read-only capabilities and the closed
schemas for its edit and test proposals, reading the package as it sits on disk.

`execution.py` runs the other half: it compiles `proposals.ts` and executes it
through the shipped `apxm execute-canonical`, so the two Capabilities this
package ships are checked by running their handlers rather than by reading what
the manifest says about them.
