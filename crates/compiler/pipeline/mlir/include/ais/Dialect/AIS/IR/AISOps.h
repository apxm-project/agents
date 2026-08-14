/**
 * @file  AISOps.h
 * @brief Operation class prototypes for the AIS dialect.
 *
 * The header pulls in the generated operation classes through
 * `GET_OP_CLASSES`; the matching `#undef` keeps the macro from leaking
 * into later includes.
 */

#ifndef APXM_AIS_OPS_H
#define APXM_AIS_OPS_H

#include "ais/Dialect/AIS/IR/AISTypes.h"
#include "ais/Dialect/AIS/IR/AISAttributes.h"
#include "mlir/Bytecode/BytecodeOpInterface.h"
#include "mlir/IR/BuiltinOps.h"
#include "mlir/IR/OpDefinition.h"
#include "mlir/IR/OpImplementation.h"
#include "mlir/IR/Operation.h"
#include "mlir/Interfaces/SideEffectInterfaces.h"
#include "mlir/Support/TypeID.h"
#include "llvm/ADT/StringRef.h"

#define GET_OP_CLASSES
#include "ais/Dialect/AIS/IR/AISOps.h.inc"
#undef GET_OP_CLASSES

#endif  // APXM_AIS_OPS_H
