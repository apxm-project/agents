/**
 * @file  AISTypes.cpp
 * @brief Runtime implementation of the AIS type system.
 *
 * Provides the body of every out-of-line method declared in AISTypes.h:
 *   - `get` constructors that uniquify the type in the MLIRContext
 *   - String ↔ enum conversion helpers for MemorySpace
 *   - `getDefaultHandlePayloadType` helper for legacy Handle payloads
 *
 * The file is intentionally free of dialect or operation logic; it only
 * realises the low-level type storage manipulation required by the context.
 */

#include "ais/Dialect/AIS/IR/AISTypes.h"
#include "mlir/IR/BuiltinTypes.h"
#include "llvm/ADT/StringSwitch.h"
#include <cassert>
#include <utility>

using namespace mlir;
using namespace mlir::ais;

Type mlir::ais::getDefaultHandlePayloadType(MLIRContext *context) {
  return NoneType::get(context);
}

std::optional<MemorySpace> mlir::ais::symbolizeMemorySpace(StringRef value) {
  return llvm::StringSwitch<std::optional<MemorySpace>>(value)
      .CaseLower("stm", MemorySpace::STM)
      .CaseLower("ltm", MemorySpace::LTM)
      .CaseLower("episodic", MemorySpace::Episodic)
      .Default(std::nullopt);
}

StringRef mlir::ais::stringifyMemorySpace(MemorySpace space) {
  switch (space) {
  case MemorySpace::STM:
    return "stm";
  case MemorySpace::LTM:
    return "ltm";
  case MemorySpace::Episodic:
    return "episodic";
  }
  llvm_unreachable("Unknown memory space");
}

TypeRefType TypeRefType::get(MLIRContext *context, StringRef typeRef) {
  assert(!typeRef.empty() && "AIS type references must be non-empty");
  return Base::get(context, typeRef);
}

StringRef TypeRefType::getTypeRef() const {
  return getImpl()->typeRef;
}

TokenType TokenType::get(MLIRContext *context, TypeRefType typeRef) {
  assert(typeRef && "AIS tokens require a TypeRefType payload");
  return Base::get(context, typeRef);
}

TypeRefType TokenType::getInnerType() const {
  return getImpl()->typeRef;
}

HandleType HandleType::get(MLIRContext *context, MemorySpace space, Type payload) {
  if (!payload)
    payload = getDefaultHandlePayloadType(context);
  return Base::get(context, std::make_pair(space, payload));
}

MemorySpace HandleType::getSpace() const {
  return getImpl()->space;
}

Type HandleType::getPayload() const {
  return getImpl()->payload;
}

GoalType GoalType::get(MLIRContext *context, unsigned priority) {
  return Base::get(context, priority);
}

unsigned GoalType::getPriority() const {
  return getImpl()->priority;
}
