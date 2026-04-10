# Add subdirectories with clear separation of concerns
# Use unique binary directories to avoid CMake conflicts

if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/Dialect/CMakeLists.txt")
  add_subdirectory(Dialect ${CMAKE_CURRENT_BINARY_DIR}/Dialect-build)
endif()

if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/CAPI/CMakeLists.txt")
  add_subdirectory(CAPI ${CMAKE_CURRENT_BINARY_DIR}/CAPI-build)
endif()
