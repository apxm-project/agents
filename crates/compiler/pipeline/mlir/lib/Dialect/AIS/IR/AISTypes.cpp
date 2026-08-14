/**
 * @file  AISTypes.cpp
 * @brief Runtime implementation of the AIS type system.
 *
 * Provides the body of every out-of-line method declared in AISTypes.h:
 *   - `get` constructors that uniquify the type in the MLIRContext
 *
 * The file is intentionally free of dialect or operation logic; it only
 * realises the low-level type storage manipulation required by the context.
 */

#include "ais/Dialect/AIS/IR/AISTypes.h"
#include "mlir/IR/BuiltinTypes.h"
#include <cassert>

using namespace mlir;
using namespace mlir::ais;

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
