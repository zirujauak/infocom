## Components

### Common

`InfocomError`: general error type returned in `Result`s.
### Instruction

Functions and structures for instructions and instruction execution.

```
fn decode_instruction(&FrameStack, usize) -> Result<Opcode,Instruction>
```

Decodes the instruction at `address`, returning an Instruction:

```
struct Instruction {
    address: usize,
    form: OpcodeForm,
    opcode: u8,
    operand_types: Vec<OperandType>,
    operands: Vec<u16>,
    store_variable: Option<u8>,
    branch_offset: Option<BranchOffset>,
    next_pc: usize
}
```

Constant operands are stored by value, variable operands store the variable number and will need to be dereferenced at execution.  Store instructions include a `store_variable`.  Branch instructions include a `branch_offset`.  `next_pc` is the address of the next instruction, taking into account literal string data for the `PRINT` and `PRINT_RET` instructions (assuming any branch condition is not met).

### Memory

The Z-Machine memory map.

The following functions implement memory regions - read is limited to static memory (up to address $FFFF) and writes are limited to dynamic memory below the static memory mark in the header.

```
fn get_byte(usize) -> Result<u8,InfocomError>
fn get_word(usize) -> Result<u16,InfocomError>
fn write_byte(usize, u8) -> Result<(),InfocomError>
fn write_word(usize, u16) -> Result<(),InfocomError>
```
A read-only copy of the full memory map can be obtained via `get_memory`, which is useful for reading instructions, routines, and string data that exists in high memory.

### Object Table

Structs representing Z-Machine objects.

The `ObjectTable` struct is the public interface to the object table.

```
ObjectTable::new(&mut MemoryMap) -> Result<ObjectTable,InfocomError>
```

Objects can be retrieved from the table:
```
ObjectTable.get_object(usize) -> Result<Object,InfocomError>
```
Within `Object` is the `PropertyTable` with a list of `Property` objects representing the properties of the object, the `attributes` flags, and references to `parent`, `sibling`, and `child` objects.  Changes to `Object` (including the associated `PropertyTable`) are stored to a `MemoryMap` via the `save` function.

### State

Structs and functions related to the runtime state of the Z-Machine.

The current execution context is represented by a `Frame` as contained within the `FrameStack`.  Reference to variables should be directed through the `FrameStack`, which will delegate to the runtime stack, local routine variables, and global variables with appropriate error checking.  A new execution context is created via the `call` function, which decodes a routine header from member to generate a new context.  Returning from a routine is handled by `return_from`, returning execution to the instruction following the original `call` and storing the return value as needed.

### Text

Functions related to the decoding of ZSCII to text and the encoding of text to ZSCII dictionary entries.

### Session

Structs and functions related to session management for the microservice REST architecture.

### Redis Connection

Functions to store and retrieve structs to and from a Redis cache.  Used for the experimental microservice REST architecture.