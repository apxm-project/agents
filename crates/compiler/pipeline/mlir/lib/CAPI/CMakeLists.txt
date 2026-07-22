# Collect CAPI-specific sources
file(GLOB APXM_CAPI_SOURCES CONFIGURE_DEPENDS
  ${CMAKE_CURRENT_SOURCE_DIR}/*.cpp
)
list(REMOVE_ITEM APXM_CAPI_SOURCES ${CMAKE_CURRENT_SOURCE_DIR}/CodeGen.cpp)


add_library(apxm_compiler_c SHARED ${APXM_CAPI_SOURCES})

target_compile_definitions(apxm_compiler_c
  PRIVATE
  APXM_CAPI_INTERNAL
)

target_include_directories(apxm_compiler_c
  PUBLIC
  ${APXM_INCLUDE_DIR}
  ${APXM_GENERATED_INCLUDE_DIR}
  PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}
  ${LLVM_INCLUDE_DIRS}
  ${MLIR_INCLUDE_DIRS}
)

target_link_libraries(apxm_compiler_c
  PRIVATE
  APXMAISIR
  MLIRArithDialect
  MLIRIR
  MLIRFuncDialect
  MLIRParser
  MLIRPass
  MLIRSCFDialect
  MLIRSupport
  MLIRTransforms
)

if(APXM_RUNTIME_RPATHS)
  set_target_properties(apxm_compiler_c
    PROPERTIES
      BUILD_RPATH "${APXM_RUNTIME_RPATHS}"
      INSTALL_RPATH "${APXM_RUNTIME_RPATHS}"
  )
endif()

install(TARGETS apxm_compiler_c
  LIBRARY DESTINATION ${CMAKE_INSTALL_PREFIX}/lib
  RUNTIME DESTINATION ${CMAKE_INSTALL_PREFIX}/bin
)
