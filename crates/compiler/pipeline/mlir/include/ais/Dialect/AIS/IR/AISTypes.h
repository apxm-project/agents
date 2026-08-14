/**
 * @file  AISTypes.h
 * @brief Type interface and uniqued storage for the AIS dialect.
 *
 * Defines the two first-class types exported by the dialect:
 *   - TypeRefType – dialect-owned reference to a source or compiler type
 *   - TokenType  – data-flow token carrying a required TypeRefType payload
 *
 * Storage classes live in the private `detail` namespace and conform to
 * MLIR's TypeStorage contract so that identical types are uniqued in the
 * context.  Public classes expose only immutable accessors; construction
 * is handled by the static `get` helpers.
 */

#ifndef APXM_AIS_TYPES_H
#define APXM_AIS_TYPES_H

#include "mlir/IR/BuiltinTypes.h"
#include "mlir/IR/TypeSupport.h"
#include "mlir/IR/Types.h"
#include "llvm/ADT/StringRef.h"

namespace mlir::ais {

//===----------------------------------------------------------------------===//
// Forward declarations (public)
//===----------------------------------------------------------------------===//

class TypeRefType;
class TokenType;

//===----------------------------------------------------------------------===//
// Storage implementation (private to the dialect)
//===----------------------------------------------------------------------===//

namespace detail {

struct TypeRefTypeStorage final : public TypeStorage {
  using KeyTy = StringRef;

  explicit TypeRefTypeStorage(StringRef typeRef) : typeRef(typeRef) {}

  bool operator==(const KeyTy &key) const { return key == typeRef; }

  static TypeRefTypeStorage *construct(TypeStorageAllocator &alloc, const KeyTy &key) {
    return new (alloc.allocate<TypeRefTypeStorage>()) TypeRefTypeStorage(alloc.copyInto(key));
  }

  StringRef typeRef;
};

} // namespace detail

//===----------------------------------------------------------------------===//
// Public type classes
//===----------------------------------------------------------------------===//

/// A stable, dialect-owned reference to one exact source or compiler type.
class TypeRefType : public Type::TypeBase<TypeRefType, Type, detail::TypeRefTypeStorage> {
public:
  using Base::Base;

  static constexpr StringLiteral name = "ais.type_ref";

  static TypeRefType get(MLIRContext *ctx, StringRef typeRef);
  StringRef getTypeRef() const;
};

namespace detail {

struct TokenTypeStorage final : public TypeStorage {
  using KeyTy = TypeRefType;

  explicit TokenTypeStorage(TypeRefType typeRef) : typeRef(typeRef) {}

  bool operator==(const KeyTy &key) const { return key == typeRef; }

  static TokenTypeStorage *construct(TypeStorageAllocator &alloc, const KeyTy &key) {
    return new (alloc.allocate<TokenTypeStorage>()) TokenTypeStorage(key);
  }

  TypeRefType typeRef;
};

} // namespace detail

class TokenType : public Type::TypeBase<TokenType, Type, detail::TokenTypeStorage> {
public:
  using Base::Base;

  static constexpr StringLiteral name = "ais.token";

  static TokenType get(MLIRContext *ctx, TypeRefType typeRef);
  TypeRefType getInnerType() const;
};

} // namespace mlir::ais

#endif // APXM_AIS_TYPES_H
