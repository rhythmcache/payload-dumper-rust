
#define PAYLOAD_DUMPER_MAJOR 0
#define PAYLOAD_DUMPER_MINOR 8
#define PAYLOAD_DUMPER_PATCH 0

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * status codes for progress callback
 */
#define STATUS_STARTED 0

#define STATUS_IN_PROGRESS 1

#define STATUS_COMPLETED 2

#define STATUS_WARNING 3

#define SUPPORTED_PAYLOAD_VERSION 2

/**
 * progress callback function type
 *
 * @param user_data User-provided data pointer
 * @param partition_name Name of the partition being extracted (temporary pointer)
 * @param current_operation Current operation number (0-based)
 * @param total_operations Total number of operations
 * @param percentage Completion percentage (0.0 to 100.0)
 * @param status Status code (see STATUS_* constants)
 * @param warning_message Warning message if status is STATUS_WARNING (temporary pointer)
 * @return non-zero to continue extraction, 0 to cancel
 */
typedef int32_t (*CProgressCallback)(void* user_data,
                                     const char* partition_name,
                                     uint64_t current_operation,
                                     uint64_t total_operations,
                                     double percentage,
                                     int32_t status,
                                     const char* warning_message);

/**
 * get the last error message
 * returns NULL if no error occurred
 * the returned string is valid until the next call from the same thread
 *
 * point: errors are thread-local. Each thread maintains its own error state.
 */
const char* payload_get_last_error(void);

/**
 * clear the last error
 */
void payload_clear_error(void);

/**
 * free a string allocated by this library
 */
void payload_free_string(char* s);

/**
 * list all partitions in a payload.bin file
 * Returns a JSON string on success, NULL on failure
 * the caller must free the returned string with payload_free_string()
 *
 * the returned JSON structure:
 * {
 *   "partitions": [...],
 *   "total_partitions": 10,
 *   "total_operations": 1000,
 *   "total_size_bytes": 5000000000,
 *   "total_size_readable": "4.66 GB"
 * }
 */
char* payload_list_partitions(const char* payload_path);

/**
 * list all partitions in a ZIP file containing payload.bin
 * returns a JSON string on success, NULL on failure
 * the caller must free the returned string with payload_free_string()
 *
 * the returned JSON format is the same as payload_list_partitions()
 */
char* payload_list_partitions_zip(const char* zip_path);

/**
 * list all partitions in a remote ZIP file containing payload.bin
 * returns a JSON string on success, NULL on failure
 * the caller must free the returned string with payload_free_string()
 *
 * @param url URL to the remote ZIP file
 * @param user_agent Optional user agent string (pass NULL for default)
 * @param out_content_length Pointer to store the HTTP content length (pass NULL to ignore)
 * @return JSON string on success, NULL on failure
 *
 * the returned JSON format is the same as payload_list_partitions()
 * if out_content_length is not NULL, it will be filled with the remote file size
 */
char* payload_list_partitions_remote_zip(const char* url,
                                         const char* user_agent,
                                         uint64_t* out_content_length);

/**
 * list all partitions in a remote payload.bin file (not in ZIP)
 * returns a JSON string on success, NULL on failure
 * the caller must free the returned string with payload_free_string()
 *
 * @param url URL to the remote payload.bin file
 * @param user_agent Optional user agent string (pass NULL for default)
 * @param out_content_length Pointer to store the HTTP content length (pass NULL to ignore)
 * @return JSON string on success, NULL on failure
 *
 * the returned JSON format is the same as payload_list_partitions()
 * if out_content_length is not NULL, it will be filled with the remote file size
 */
char* payload_list_partitions_remote_bin(const char* url,
                                         const char* user_agent,
                                         uint64_t* out_content_length);

/**
 * extract a single partition from a payload.bin file
 *
 * @param payload_path Path to the payload.bin file
 * @param partition_name Name of the partition to extract
 * @param output_path Path where the partition image will be written
 * @param callback Optional progress callback (pass NULL for no callback)
 * @param user_data User data passed to callback (can be NULL)
 * @return 0 on success, -1 on failure (check payload_get_last_error())
 *
 * This function can be safely called from multiple threads concurrently.
 * Each thread can extract a different partition in parallel.
 *
 * - pass NULL for callback parameter if you don't want progress updates
 * - the partition_name and warning_message pointers passed to the callback
 *   are ONLY valid during the callback execution. Do NOT store these pointers.
 * - If you need to keep the strings, copy them immediately in the callback.
 * - do NOT call free() on these strings, they are managed by the library.
 *
 * - Return 0 from the callback to cancel extraction
 * - Return non-zero to continue
 * - cancellation may not be immediate
 */
int32_t payload_extract_partition(const char* payload_path,
                                  const char* partition_name,
                                  const char* output_path,
                                  CProgressCallback callback,
                                  void* user_data);

/**
 * extract a single partition from a ZIP file containing payload.bin
 *
 * @param zip_path Path to the ZIP file containing payload.bin
 * @param partition_name Name of the partition to extract
 * @param output_path Path where the partition image will be written
 * @param callback Optional progress callback (pass NULL for no callback)
 * @param user_data User data passed to callback (can be NULL)
 * @return 0 on success, -1 on failure (check payload_get_last_error())
 *
 * this function can be safely called from multiple threads concurrently.
 * each thread can extract a different partition in parallel.
 *
 * - pass NULL for callback parameter if you don't want progress updates
 * - the partition_name and warning_message pointers passed to the callback
 *   are ONLY valid during the callback execution. Do NOT store these pointers.
 * - if you need to keep the strings, copy them immediately in the callback.
 * - Do NOT call free() on these strings, they are managed by the library.
 *
 * - Return 0 from the callback to cancel extraction
 * - Return non-zero to continue
 * - Cancellation may not be immediate
 */
int32_t payload_extract_partition_zip(const char* zip_path,
                                      const char* partition_name,
                                      const char* output_path,
                                      CProgressCallback callback,
                                      void* user_data);

/**
 * extract a single partition from a remote ZIP file containing payload.bin
 *
 * @param url URL to the remote ZIP file
 * @param partition_name Name of the partition to extract
 * @param output_path Path where the partition image will be written
 * @param user_agent Optional user agent string (pass NULL for default)
 * @param callback Optional progress callback (pass NULL for no callback)
 * @param user_data User data passed to callback (can be NULL)
 * @return 0 on success, -1 on failure (check payload_get_last_error())
 *
 * this function can be safely called from multiple threads concurrently.
 * each thread can extract a different partition in parallel.
 *
 * - pass NULL for callback parameter if you don't want progress updates
 * - the partition_name and warning_message pointers passed to the callback
 *   are ONLY valid during the callback execution. Do NOT store these pointers.
 * - if you need to keep the strings, copy them immediately in the callback.
 * - Do NOT call free() on these strings, they are managed by the library.
 *
 * - Return 0 from the callback to cancel extraction
 * - Return non-zero to continue
 * - Cancellation may not be immediate
 */
int32_t payload_extract_partition_remote_zip(const char* url,
                                             const char* partition_name,
                                             const char* output_path,
                                             const char* user_agent,
                                             CProgressCallback callback,
                                             void* user_data);

/**
 * extract a single partition from a remote payload.bin file (not in ZIP)
 *
 * @param url URL to the remote payload.bin file
 * @param partition_name Name of the partition to extract
 * @param output_path Path where the partition image will be written
 * @param user_agent Optional user agent string (pass NULL for default)
 * @param callback Optional progress callback (pass NULL for no callback)
 * @param user_data User data passed to callback (can be NULL)
 * @return 0 on success, -1 on failure (check payload_get_last_error())
 *
 * this function can be safely called from multiple threads concurrently.
 * each thread can extract a different partition in parallel.
 *
 * - pass NULL for callback parameter if you don't want progress updates
 * - the partition_name and warning_message pointers passed to the callback
 *   are ONLY valid during the callback execution. Do NOT store these pointers.
 * - if you need to keep the strings, copy them immediately in the callback.
 * - Do NOT call free() on these strings, they are managed by the library.
 *
 * - Return 0 from the callback to cancel extraction
 * - Return non-zero to continue
 * - Cancellation may not be immediate
 */
int32_t payload_extract_partition_remote_bin(const char* url,
                                             const char* partition_name,
                                             const char* output_path,
                                             const char* user_agent,
                                             CProgressCallback callback,
                                             void* user_data);

/**
 * get library version
 * returns a static string, do not free
 */
const char* payload_get_version(void);

/**
 * initialize the library (optional, but recommended for thread safety)
 * should be called once before any other library functions
 * @return 0 on success, -1 on failure
 */
int32_t payload_init(void);

/**
 * cleanup library resources
 * should be called once when done using the library
 * no library functions should be called after this
 */
void payload_cleanup(void);
