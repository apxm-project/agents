/**
 * @file AISDialect.cpp
 * @brief MLIR dialect implementation for the AIS dialect.
 *
 * This file registers the AIS dialect, its custom types (TypeRef, Token),
 * attributes, and the associated parser/printer logic.
 */

#include "ais/Dialect/AIS/IR/AISDialect.h"
#include "ais/Dialect/AIS/IR/AISAttributes.h"
#include "mlir/IR/Builders.h"
#include "mlir/IR/DialectImplementation.h"
#include "llvm/ADT/TypeSwitch.h"
#include <string>

using namespace mlir;
using namespace mlir::ais;

#include "ais/Dialect/AIS/IR/AISEnums.cpp.inc"
#define GET_ATTRDEF_CLASSES
#include "ais/Dialect/AIS/IR/AISAttributes.cpp.inc"

void AISDialect::initialize() {
  addOperations<
#define GET_OP_LIST
#include "ais/Dialect/AIS/IR/AISOps.cpp.inc"
      >();
  addAttributes<
#define GET_ATTRDEF_LIST
#include "ais/Dialect/AIS/IR/AISAttributes.cpp.inc"
      >();
  addTypes<TypeRefType, TokenType>();
}

Type AISDialect::parseType(DialectAsmParser &parser) const {

  MLIRContext *ctx = getContext();
  StringRef keyword;

  if (failed(parser.parseKeyword(&keyword)))
    return Type();

  auto parseToken = [&]() -> Type {
    if (parser.parseLess())
      return Type();

    Type payload;
    if (parser.parseType(payload) || parser.parseGreater())
      return Type();

    auto typeRef = dyn_cast<TypeRefType>(payload);
    if (!typeRef) {
      parser.emitError(parser.getCurrentLocation())
          << "AIS token payload must be an !ais.type_ref type";
      return Type();
    }
    return TokenType::get(ctx, typeRef);
  };

  auto parseTypeRef = [&]() -> Type {
    if (parser.parseLess())
      return Type();

    std::string typeRef;
    if (parser.parseString(&typeRef) || parser.parseGreater())
      return Type();
    if (typeRef.empty()) {
      parser.emitError(parser.getCurrentLocation())
          << "AIS type references must be non-empty";
      return Type();
    }
    return TypeRefType::get(ctx, typeRef);
  };

  if (keyword == "type_ref")
    return parseTypeRef();
  if (keyword == "token")
    return parseToken();

  parser.emitError(parser.getCurrentLocation()) << "unknown AIS type \"" << keyword << "\"";
  return Type();
}

void AISDialect::printType(Type type, DialectAsmPrinter &printer) const {
  TypeSwitch<Type>(type)
      .Case<TypeRefType>([&](TypeRefType typeRef) {
        printer << "type_ref<";
        printer.printString(typeRef.getTypeRef());
        printer << '>';
      })
      .Case<TokenType>([&](TokenType token) {
        printer << "token<";
        printer.printType(token.getInnerType());
        printer << '>';
      })
      .Default([](Type) { llvm_unreachable("unknown AIS type"); });
}

#include "ais/Dialect/AIS/IR/AISDialect.cpp.inc"
