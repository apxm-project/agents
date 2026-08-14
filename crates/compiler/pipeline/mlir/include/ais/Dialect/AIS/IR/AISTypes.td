/**
 * @file  AISTypes.td
 * @brief Type definitions and type constraints for the AIS dialect.
 *
 * Two first-class types are exposed:
 *   - !ais.type_ref<"Type"> – dialect-owned source/compiler type identity
 *   - !ais.token<!ais.type_ref<"Type">> – data-flow token carrying a typed value
 */

#ifndef APXM_AIS_TYPES
#define APXM_AIS_TYPES

include "ais/Dialect/AIS/IR/AISDialect.td"

//===----------------------------------------------------------------------===//
// AIS Type Definitions
//===----------------------------------------------------------------------===//

/// TypeRefType - Identifies the exact type carried by an AIS token.
def AIS_TypeRefType : DialectType<AIS_Dialect,
    CPred<"::llvm::isa<::mlir::ais::TypeRefType>($_self)">,
    "AIS type reference", "::mlir::ais::TypeRefType"> {
  let summary = "Dialect-owned source or compiler type reference";
}

/// TokenType - Represents a dataflow token carrying an exact typed value.
def AIS_TokenType : DialectType<AIS_Dialect,
    CPred<"::llvm::isa<::mlir::ais::TokenType>($_self)">,
    "AIS token type", "::mlir::ais::TokenType"> {
  let summary = "Token carrying a typed value";
  let description = [{
    TokenType represents a dataflow token in the AIS execution model. Every
    token carries an `!ais.type_ref` payload and establishes a typed data
    dependency between operations.
  }];
}

#endif // APXM_AIS_TYPES
