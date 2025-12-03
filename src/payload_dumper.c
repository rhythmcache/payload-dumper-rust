#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <direct.h>
#include <io.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <windows.h>
#include <winsock2.h>
#include <ws2tcpip.h>
#pragma comment(lib, "ws2_32.lib")
#define mkdir(path, mode) _mkdir(path)
#define sleep(seconds) Sleep((seconds) * 1000)
#define strdup _strdup
#define ntohl(x) _byteswap_ulong(x)
#define be64toh(x) _byteswap_uint64(x)
HANDLE hConsole;
CONSOLE_SCREEN_BUFFER_INFO consoleInfo;
#else
#define _GNU_SOURCE
#define _DEFAULT_SOURCE
#include <arpa/inet.h>
#include <pthread.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <unistd.h>
#endif

#include <bzlib.h>
#include <errno.h>
#include <fcntl.h>
#include <lzma.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <zstd.h>

#include "update_metadata.pb-c.h"
#include "zip_parser.h"

#ifdef ENABLE_HTTP_SUPPORT
#include "http_reader.h"
#endif

#ifdef _WIN32
#define PRIu64 "llu"
#else
#include <inttypes.h>
#endif

#ifdef _WIN32
typedef HANDLE thread_t;
typedef CRITICAL_SECTION mutex_t;
#define MUTEX_INITIALIZER {0}
#else
typedef pthread_t thread_t;
typedef pthread_mutex_t mutex_t;
#define MUTEX_INITIALIZER PTHREAD_MUTEX_INITIALIZER
#endif

int get_terminal_width(void) {
#ifdef _WIN32
  CONSOLE_SCREEN_BUFFER_INFO csbi;
  if (GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &csbi)) {
    return csbi.srWindow.Right - csbi.srWindow.Left + 1;
  }
  return 80; // default fallback
#else
  struct winsize w;
  if (ioctl(STDOUT_FILENO, TIOCGWINSZ, &w) == 0 && w.ws_col > 0) {
    return w.ws_col;
  }
  return 80; // default fallback
#endif
}

void mutex_init(mutex_t *mutex);
void mutex_destroy(mutex_t *mutex);
void mutex_lock(mutex_t *mutex);
void mutex_unlock(mutex_t *mutex);
int thread_create(thread_t *thread, void *(*start_routine)(void *), void *arg);
void thread_join(thread_t thread);

#define MAGIC_HEADER "CrAU"
#define MAGIC_LEN 4
#define MAX_PARTITIONS 64
#define MAX_THREADS 8

typedef struct {
  char partition_name[256];
  size_t total_ops;
  size_t completed_ops;
  int thread_id;
} progress_info_t;

typedef struct {
  reader_t *payload_reader;
  uint64_t data_offset;
  uint32_t block_size;
  char *out_dir;
  mutex_t *reader_mutex;
} thread_data_t;

uint32_t read_u32_be(const uint8_t *data);
uint64_t read_u64_be(const uint8_t *data);
void update_progress(int partition_idx);

int decompress_lzma(const uint8_t *compressed, size_t comp_size, FILE *out_file,
                    int64_t write_offset);
int decompress_zstd(const uint8_t *compressed, size_t comp_size, FILE *out_file,
                    int64_t write_offset);
int decompress_bz2(const uint8_t *compressed, size_t comp_size, FILE *out_file,
                   int64_t write_offset);
int process_operation(ChromeosUpdateEngine__InstallOperation *op,
                      reader_t *payload_reader, FILE *out_file,
                      uint64_t data_offset, uint32_t block_size,
                      mutex_t *reader_mutex);
ChromeosUpdateEngine__PartitionUpdate *get_next_partition(int *partition_idx);
void *process_partition_thread(void *arg);
void list_partitions(ChromeosUpdateEngine__DeltaArchiveManifest *manifest);
reader_t *open_payload_source(const char *source_path, const char *user_agent,
                              uint64_t *payload_offset, uint64_t *payload_size);
int extract_payload(const char *payload_path, const char *user_agent,
                    const char *out_dir, const char *images_list, int list_only,
                    int num_threads);
void print_usage(const char *program_name);

#ifdef ENABLE_HTTP_SUPPORT
char *format_size(uint64_t bytes);
#else

char *format_size(uint64_t bytes) {
  static char buffer[32];
  const char *units[] = {"B", "KB", "MB", "GB", "TB"};
  int unit_idx = 0;
  double size = (double)bytes;

  while (size >= 1024.0 && unit_idx < 4) {
    size /= 1024.0;
    unit_idx++;
  }

  snprintf(buffer, sizeof(buffer), "%.2f %s", size, units[unit_idx]);
  return buffer;
}
#endif

void mutex_init(mutex_t *mutex) {
#ifdef _WIN32
  InitializeCriticalSection(mutex);
#else
  pthread_mutex_init(mutex, NULL);
#endif
}

void mutex_destroy(mutex_t *mutex) {
#ifdef _WIN32
  DeleteCriticalSection(mutex);
#else
  pthread_mutex_destroy(mutex);
#endif
}

void mutex_lock(mutex_t *mutex) {
#ifdef _WIN32
  EnterCriticalSection(mutex);
#else
  pthread_mutex_lock(mutex);
#endif
}

void mutex_unlock(mutex_t *mutex) {
#ifdef _WIN32
  LeaveCriticalSection(mutex);
#else
  pthread_mutex_unlock(mutex);
#endif
}

#ifdef _WIN32
DWORD WINAPI windows_thread_wrapper(LPVOID arg) {
  void *(*start_routine)(void *) = (void *(*)(void *))((void **)arg)[0];
  void *thread_arg = ((void **)arg)[1];
  start_routine(thread_arg);
  free(arg);
  return 0;
}
#endif

int thread_create(thread_t *thread, void *(*start_routine)(void *), void *arg) {
#ifdef _WIN32
  void **wrapper_args = malloc(2 * sizeof(void *));
  if (!wrapper_args)
    return -1;
  wrapper_args[0] = (void *)start_routine;
  wrapper_args[1] = arg;
  *thread =
      CreateThread(NULL, 0, windows_thread_wrapper, wrapper_args, 0, NULL);
  return (*thread == NULL) ? -1 : 0;
#else
  return pthread_create(thread, NULL, start_routine, arg);
#endif
}

void thread_join(thread_t thread) {
#ifdef _WIN32
  WaitForSingleObject(thread, INFINITE);
  CloseHandle(thread);
#else
  pthread_join(thread, NULL);
#endif
}

progress_info_t g_progress[MAX_PARTITIONS];
int g_num_partitions = 0;
mutex_t g_progress_mutex;

ChromeosUpdateEngine__PartitionUpdate **g_work_queue = NULL;
int g_queue_size = 0;
int g_current_work_index = 0;
mutex_t g_queue_mutex;

uint32_t read_u32_be(const uint8_t *data) {
  uint32_t value;
  memcpy(&value, data, sizeof(value));
  return ntohl(value);
}

uint64_t read_u64_be(const uint8_t *data) {
  uint64_t value;
  memcpy(&value, data, sizeof(value));
  return be64toh(value);
}

static int progress_initialized = 0;

void update_progress(int partition_idx) {
  mutex_lock(&g_progress_mutex);
  g_progress[partition_idx].completed_ops++;

  int term_width = get_terminal_width();
  int bar_width = (term_width > 80) ? 30 : (term_width > 60) ? 20 : 10;
  int name_width = (term_width > 100) ? 20 : (term_width > 80) ? 15 : 12;

  if (!progress_initialized) {
    printf("\n");
#ifdef _WIN32
    hConsole = GetStdHandle(STD_OUTPUT_HANDLE);
    GetConsoleScreenBufferInfo(hConsole, &consoleInfo);
#endif
    for (int i = 0; i < g_num_partitions; i++) {
      progress_info_t *p = &g_progress[i];
      printf("[T%d] %-*s [%*s] %3d%% (%zu/%zu)\n", p->thread_id, name_width,
             p->partition_name, bar_width, "", 0, (size_t)0, p->total_ops);
    }
    progress_initialized = 1;
  }

#ifdef _WIN32
  GetConsoleScreenBufferInfo(hConsole, &consoleInfo);
  COORD newPos = {0,
                  (SHORT)(consoleInfo.dwCursorPosition.Y - g_num_partitions)};
  if (newPos.Y < 0)
    newPos.Y = 0;
  SetConsoleCursorPosition(hConsole, newPos);
#else
  printf("\033[%dA", g_num_partitions);
#endif

  for (int i = 0; i < g_num_partitions; i++) {
    progress_info_t *p = &g_progress[i];
    int percent =
        (int)((double)p->completed_ops / (double)p->total_ops * 100.0);
    int filled =
        (int)((double)p->completed_ops / (double)p->total_ops * bar_width);

#ifdef _WIN32
    printf("%-*s\r", term_width, "");
#else
    printf("\033[2K");
#endif

    printf("[T%d] %-*.*s [", p->thread_id, name_width, name_width,
           p->partition_name);
    for (int j = 0; j < bar_width; j++) {
      if (j < filled)
        printf("=");
      else if (j == filled && p->completed_ops < p->total_ops)
        printf(">");
      else
        printf(" ");
    }
    printf("] %3d%% (%zu/%zu)", percent, p->completed_ops, p->total_ops);
    if (p->completed_ops == p->total_ops) {
#ifdef _WIN32
      printf(" [DONE]");
#else
      printf(" ✓ DONE");
#endif
    }
    printf("\n");
  }
  fflush(stdout);
  mutex_unlock(&g_progress_mutex);
}

int decompress_lzma(const uint8_t *compressed, size_t comp_size, FILE *out_file,
                    int64_t write_offset) {
  lzma_stream strm = LZMA_STREAM_INIT;
  lzma_ret ret = lzma_stream_decoder(&strm, UINT64_MAX, 0);
  if (ret != LZMA_OK)
    return -1;

  uint8_t out_buf[8192];
  strm.next_in = compressed;
  strm.avail_in = comp_size;

#ifdef _WIN32
  _fseeki64(out_file, write_offset, SEEK_SET);
#else
  fseek(out_file, write_offset, SEEK_SET);
#endif

  while (1) {
    strm.next_out = out_buf;
    strm.avail_out = sizeof(out_buf);

    ret = lzma_code(&strm, LZMA_FINISH);

    size_t write_size = sizeof(out_buf) - strm.avail_out;
    if (write_size > 0) {
      fwrite(out_buf, 1, write_size, out_file);
    }

    if (ret == LZMA_STREAM_END) {
      lzma_end(&strm);
      return 0;
    } else if (ret != LZMA_OK) {
      lzma_end(&strm);
      return -1;
    }
  }
}

int decompress_zstd(const uint8_t *compressed, size_t comp_size, FILE *out_file,
                    int64_t write_offset) {
  ZSTD_DCtx *dctx = ZSTD_createDCtx();
  if (!dctx)
    return -1;

  uint8_t out_buf[8192];
  ZSTD_inBuffer input = {compressed, comp_size, 0};

#ifdef _WIN32
  _fseeki64(out_file, write_offset, SEEK_SET);
#else
  fseek(out_file, write_offset, SEEK_SET);
#endif

  while (input.pos < input.size) {
    ZSTD_outBuffer output = {out_buf, sizeof(out_buf), 0};
    size_t ret = ZSTD_decompressStream(dctx, &output, &input);

    if (ZSTD_isError(ret)) {
      ZSTD_freeDCtx(dctx);
      return -1;
    }

    if (output.pos > 0) {
      fwrite(out_buf, 1, output.pos, out_file);
    }
  }

  ZSTD_freeDCtx(dctx);
  return 0;
}

int decompress_bz2(const uint8_t *compressed, size_t comp_size, FILE *out_file,
                   int64_t write_offset) {
  bz_stream strm = {0};
  int ret = BZ2_bzDecompressInit(&strm, 0, 0);
  if (ret != BZ_OK)
    return -1;

  uint8_t out_buf[8192];
  strm.next_in = (char *)(uintptr_t)compressed;
  strm.avail_in = (unsigned int)comp_size;

#ifdef _WIN32
  _fseeki64(out_file, write_offset, SEEK_SET);
#else
  fseek(out_file, write_offset, SEEK_SET);
#endif

  while (1) {
    strm.next_out = (char *)out_buf;
    strm.avail_out = (unsigned int)sizeof(out_buf);

    ret = BZ2_bzDecompress(&strm);

    size_t write_size = sizeof(out_buf) - strm.avail_out;
    if (write_size > 0) {
      fwrite(out_buf, 1, write_size, out_file);
    }

    if (ret == BZ_STREAM_END) {
      BZ2_bzDecompressEnd(&strm);
      return 0;
    } else if (ret != BZ_OK) {
      BZ2_bzDecompressEnd(&strm);
      return -1;
    }
  }
}

int process_operation(ChromeosUpdateEngine__InstallOperation *op,
                      reader_t *payload_reader, FILE *out_file,
                      uint64_t data_offset, uint32_t block_size,
                      mutex_t *reader_mutex) {

  uint8_t *op_data = NULL;
  if (op->has_data_length && op->data_length > 0) {
    op_data = malloc(op->data_length);
    if (!op_data)
      return -1;

    mutex_lock(reader_mutex);
    size_t bytes_read;
    if (reader_read_at(payload_reader, data_offset + op->data_offset, op_data,
                       op->data_length, &bytes_read) != 0 ||
        bytes_read != op->data_length) {
      mutex_unlock(reader_mutex);
      free(op_data);
      return -1;
    }
    mutex_unlock(reader_mutex);
  }

  int64_t write_offset =
      (int64_t)(op->dst_extents[0]->start_block * block_size);

  switch (op->type) {
  case CHROMEOS_UPDATE_ENGINE__INSTALL_OPERATION__TYPE__REPLACE_XZ: {
    if (decompress_lzma(op_data, op->data_length, out_file, write_offset) !=
        0) {
      free(op_data);
      return -1;
    }
    break;
  }
  case CHROMEOS_UPDATE_ENGINE__INSTALL_OPERATION__TYPE__ZSTD: {
    if (decompress_zstd(op_data, op->data_length, out_file, write_offset) !=
        0) {
      free(op_data);
      return -1;
    }
    break;
  }
  case CHROMEOS_UPDATE_ENGINE__INSTALL_OPERATION__TYPE__REPLACE_BZ: {
    if (decompress_bz2(op_data, op->data_length, out_file, write_offset) != 0) {
      free(op_data);
      return -1;
    }
    break;
  }
  case CHROMEOS_UPDATE_ENGINE__INSTALL_OPERATION__TYPE__REPLACE: {
#ifdef _WIN32
    _fseeki64(out_file, write_offset, SEEK_SET);
#else
    fseek(out_file, write_offset, SEEK_SET);
#endif
    fwrite(op_data, 1, op->data_length, out_file);
    break;
  }
  case CHROMEOS_UPDATE_ENGINE__INSTALL_OPERATION__TYPE__ZERO: {
    for (size_t i = 0; i < op->n_dst_extents; i++) {
#ifdef _WIN32
      _fseeki64(out_file,
                (__int64)(op->dst_extents[i]->start_block * block_size),
                SEEK_SET);
#else
      fseek(out_file, (long)(op->dst_extents[i]->start_block * block_size),
            SEEK_SET);
#endif
      size_t zero_size = op->dst_extents[i]->num_blocks * block_size;
      uint8_t *zero_buf = calloc(1, zero_size);
      if (zero_buf) {
        fwrite(zero_buf, 1, zero_size, out_file);
        free(zero_buf);
      }
    }
    break;
  }
  default:
    printf("\n- Unsupported operation type: %d\n", op->type);
    if (op_data)
      free(op_data);
    return -1;
  }

  if (op_data)
    free(op_data);
  return 0;
}

ChromeosUpdateEngine__PartitionUpdate *get_next_partition(int *partition_idx) {
  mutex_lock(&g_queue_mutex);
  if (g_current_work_index >= g_queue_size) {
    mutex_unlock(&g_queue_mutex);
    return NULL;
  }
  ChromeosUpdateEngine__PartitionUpdate *partition =
      g_work_queue[g_current_work_index];
  *partition_idx = g_current_work_index;
  g_current_work_index++;
  mutex_unlock(&g_queue_mutex);
  return partition;
}

void *process_partition_thread(void *arg) {
  thread_data_t *data = (thread_data_t *)arg;
  ChromeosUpdateEngine__PartitionUpdate *partition;
  int partition_idx;

  while ((partition = get_next_partition(&partition_idx)) != NULL) {
    char output_path[512];
    snprintf(output_path, sizeof(output_path), "%s/%s.img", data->out_dir,
             partition->partition_name);

    FILE *out_file = fopen(output_path, "wb");
    if (!out_file) {
      printf("Failed to create output file: %s\n", output_path);
      continue;
    }

    for (size_t i = 0; i < partition->n_operations; i++) {
      process_operation(partition->operations[i], data->payload_reader,
                        out_file, data->data_offset, data->block_size,
                        data->reader_mutex);
      update_progress(partition_idx);
    }
    fclose(out_file);
  }
  return NULL;
}

void list_partitions(ChromeosUpdateEngine__DeltaArchiveManifest *manifest) {
  int term_width = get_terminal_width();
  int name_width = (term_width > 100) ? 30 : (term_width > 80) ? 20 : 15;
  int size_width = 15;

  printf("Available partitions:\n");
  for (int i = 0; i < term_width && i < 80; i++)
    printf("─");
  printf("\n");

  printf("%-*s %-*s %-15s\n", name_width, "Partition Name", size_width, "Size",
         "Size (bytes)");

  for (int i = 0; i < term_width && i < 80; i++)
    printf("─");
  printf("\n");

  uint64_t total_size = 0;
  for (size_t i = 0; i < manifest->n_partitions; i++) {
    ChromeosUpdateEngine__PartitionUpdate *part = manifest->partitions[i];
    uint64_t max_end_block = 0;
    for (size_t j = 0; j < part->n_operations; j++) {
      ChromeosUpdateEngine__InstallOperation *op = part->operations[j];
      for (size_t k = 0; k < op->n_dst_extents; k++) {
        uint64_t end_block =
            op->dst_extents[k]->start_block + op->dst_extents[k]->num_blocks;
        if (end_block > max_end_block)
          max_end_block = end_block;
      }
    }
    uint64_t size_bytes = max_end_block * manifest->block_size;
    if (part->new_partition_info && part->new_partition_info->has_size) {
      size_bytes = part->new_partition_info->size;
    }
    total_size += size_bytes;
    printf("%-*.*s %-*s %-15" PRIu64 "\n", name_width, name_width,
           part->partition_name, size_width, format_size(size_bytes),
           size_bytes);
  }

  for (int i = 0; i < term_width && i < 80; i++)
    printf("─");
  printf("\n");

  printf("%-*s %-*s %-15" PRIu64 "\n", name_width, "Total", size_width,
         format_size(total_size), total_size);
  printf("\nTotal partitions: %zu\n", manifest->n_partitions);
  printf("Block size: %u bytes\n", manifest->block_size);
}

reader_t *open_payload_source(const char *source_path, const char *user_agent,
                              uint64_t *payload_offset,
                              uint64_t *payload_size) {
  reader_t *reader = malloc(sizeof(reader_t));
  if (!reader)
    return NULL;

#ifdef ENABLE_HTTP_SUPPORT
  if (strncmp(source_path, "http://", 7) == 0 ||
      strncmp(source_path, "https://", 8) == 0) {
    printf("- Opening remote ZIP: %s\n", source_path);
    if (reader_init_http(reader, source_path, user_agent, 0) != 0) {
      free(reader);
      return NULL;
    }
    zip_entry_t payload_entry;
    if (find_payload_entry(reader, &payload_entry) == 0) {
      if (get_data_offset(reader, &payload_entry) == 0) {
        if (verify_payload_magic(reader, payload_entry.data_offset) == 0) {
          *payload_offset = payload_entry.data_offset;
          *payload_size =
              payload_entry.uncompressed_size; // or compressed_size?
          printf("- Found payload: offset=%" PRIu64 ", size=%s\n",
                 *payload_offset, format_size(*payload_size));
          return reader;
        }
      }
    }
    reader_cleanup(reader);
    free(reader);
    return NULL;
  } else {
#else
  if (strncmp(source_path, "http://", 7) == 0 ||
      strncmp(source_path, "https://", 8) == 0) {
    printf("- Error: HTTP support is not enabled in this build.\n");
    printf(
        "- Please recompile with HTTP support enabled or use a local file.\n");
    free(reader);
    return NULL;
  } else {
#endif

    struct stat st;
    if (stat(source_path, &st) != 0) {
      free(reader);
      return NULL;
    }

    if (reader_init_file(reader, source_path) != 0) {
      free(reader);
      return NULL;
    }

    if (verify_payload_magic(reader, 0) == 0) {
      *payload_offset = 0;
      *payload_size = (uint64_t)st.st_size;
      return reader;
    }

    zip_entry_t payload_entry;
    if (find_payload_entry(reader, &payload_entry) == 0) {
      if (get_data_offset(reader, &payload_entry) == 0) {
        if (verify_payload_magic(reader, payload_entry.data_offset) == 0) {
          *payload_offset = payload_entry.data_offset;
          *payload_size =
              payload_entry.uncompressed_size; // or compressed_size?
          printf("- Found payload in ZIP: offset=%" PRIu64 ", size=%s\n",
                 *payload_offset, format_size(*payload_size));
          return reader;
        }
      }
    }
    reader_cleanup(reader);
    free(reader);
    return NULL;
  }
}

int extract_payload(const char *payload_path, const char *user_agent,
                    const char *out_dir, const char *images_list, int list_only,
                    int num_threads) {
  mutex_init(&g_progress_mutex);
  mutex_init(&g_queue_mutex);

  uint64_t payload_offset, payload_size;
  reader_t *payload_reader = open_payload_source(
      payload_path, user_agent, &payload_offset, &payload_size);
  if (!payload_reader) {
    printf("- Failed to open payload source: %s\n", payload_path);
    mutex_destroy(&g_queue_mutex);
    mutex_destroy(&g_progress_mutex);
    return -1;
  }

  uint8_t magic[MAGIC_LEN];
  size_t bytes_read;
  if (reader_read_at(payload_reader, payload_offset, magic, MAGIC_LEN,
                     &bytes_read) != 0 ||
      bytes_read != MAGIC_LEN || memcmp(magic, MAGIC_HEADER, MAGIC_LEN) != 0) {
    printf("- Invalid magic header\n");
    reader_cleanup(payload_reader);
    free(payload_reader);
    mutex_destroy(&g_queue_mutex);
    mutex_destroy(&g_progress_mutex);
    return -1;
  }

  uint8_t version_buf[8];
  if (reader_read_at(payload_reader, payload_offset + MAGIC_LEN, version_buf, 8,
                     &bytes_read) != 0 ||
      bytes_read != 8) {
    printf("- Failed to read file format version\n");
    reader_cleanup(payload_reader);
    free(payload_reader);
    mutex_destroy(&g_queue_mutex);
    mutex_destroy(&g_progress_mutex);
    return -1;
  }
  uint64_t file_format_version = read_u64_be(version_buf);

  if (file_format_version != 2) {
    printf("- Unsupported file format version: %" PRIu64 "\n",
           file_format_version);
    reader_cleanup(payload_reader);
    free(payload_reader);
    mutex_destroy(&g_queue_mutex);
    mutex_destroy(&g_progress_mutex);
    return -1;
  }

  uint8_t manifest_size_buf[8];
  if (reader_read_at(payload_reader, payload_offset + MAGIC_LEN + 8,
                     manifest_size_buf, 8, &bytes_read) != 0) {
    printf("- Failed to read manifest size\n");
    reader_cleanup(payload_reader);
    free(payload_reader);
    mutex_destroy(&g_queue_mutex);
    mutex_destroy(&g_progress_mutex);
    return -1;
  }
  uint64_t manifest_size = read_u64_be(manifest_size_buf);

  uint8_t metadata_sig_size_buf[4];
  if (reader_read_at(payload_reader, payload_offset + MAGIC_LEN + 16,
                     metadata_sig_size_buf, 4, &bytes_read) != 0) {
    printf("- Failed to read metadata signature size\n");
    reader_cleanup(payload_reader);
    free(payload_reader);
    mutex_destroy(&g_queue_mutex);
    mutex_destroy(&g_progress_mutex);
    return -1;
  }
  uint32_t metadata_signature_size = read_u32_be(metadata_sig_size_buf);

  uint8_t *manifest_data = malloc(manifest_size);
  if (!manifest_data ||
      reader_read_at(payload_reader, payload_offset + MAGIC_LEN + 20,
                     manifest_data, manifest_size, &bytes_read) != 0 ||
      bytes_read != manifest_size) {
    printf("- Failed to read manifest\n");
    free(manifest_data);
    reader_cleanup(payload_reader);
    free(payload_reader);
    mutex_destroy(&g_queue_mutex);
    mutex_destroy(&g_progress_mutex);
    return -1;
  }

  uint64_t data_offset =
      payload_offset + MAGIC_LEN + 20 + manifest_size + metadata_signature_size;

  ChromeosUpdateEngine__DeltaArchiveManifest *manifest =
      chromeos_update_engine__delta_archive_manifest__unpack(
          NULL, manifest_size, manifest_data);

  if (!manifest) {
    printf("- Failed to parse manifest\n");
    free(manifest_data);
    reader_cleanup(payload_reader);
    free(payload_reader);
    mutex_destroy(&g_queue_mutex);
    mutex_destroy(&g_progress_mutex);
    return -1;
  }

  if (list_only) {
    list_partitions(manifest);
    chromeos_update_engine__delta_archive_manifest__free_unpacked(manifest,
                                                                  NULL);
    free(manifest_data);
    reader_cleanup(payload_reader);
    free(payload_reader);
    mutex_destroy(&g_queue_mutex);
    mutex_destroy(&g_progress_mutex);
    return 0;
  }

#ifdef _WIN32
  _mkdir(out_dir);
#else
  mkdir(out_dir, 0755);
#endif

  g_num_partitions = (int)manifest->n_partitions;
  for (size_t i = 0; i < manifest->n_partitions; i++) {
#ifdef _MSC_VER
    strncpy_s(g_progress[i].partition_name,
              sizeof(g_progress[i].partition_name),
              manifest->partitions[i]->partition_name, _TRUNCATE);
#else
    strncpy(g_progress[i].partition_name,
            manifest->partitions[i]->partition_name,
            sizeof(g_progress[i].partition_name) - 1);
    g_progress[i].partition_name[sizeof(g_progress[i].partition_name) - 1] =
        '\0';
#endif
    g_progress[i].total_ops = manifest->partitions[i]->n_operations;
    g_progress[i].completed_ops = 0;
    g_progress[i].thread_id = (int)(i % (size_t)num_threads);
  }

  thread_t threads[MAX_THREADS];
  thread_data_t thread_data[MAX_THREADS];
  mutex_t reader_mutex;
  mutex_init(&reader_mutex);

  g_work_queue = malloc(manifest->n_partitions *
                        sizeof(ChromeosUpdateEngine__PartitionUpdate *));
  g_queue_size = 0;
  g_current_work_index = 0;

  for (size_t i = 0; i < manifest->n_partitions; i++) {
    if (images_list && strlen(images_list) > 0) {
      if (!strstr(images_list, manifest->partitions[i]->partition_name))
        continue;
    }
    g_work_queue[g_queue_size] = manifest->partitions[i];
    g_queue_size++;
  }

  progress_initialized = 0;
  g_num_partitions = g_queue_size;
  for (int i = 0; i < g_queue_size; i++) {
#ifdef _MSC_VER
    strncpy_s(g_progress[i].partition_name,
              sizeof(g_progress[i].partition_name),
              g_work_queue[i]->partition_name, _TRUNCATE);
#else
    strncpy(g_progress[i].partition_name, g_work_queue[i]->partition_name,
            sizeof(g_progress[i].partition_name) - 1);
    g_progress[i].partition_name[sizeof(g_progress[i].partition_name) - 1] =
        '\0';
#endif
    g_progress[i].total_ops = g_work_queue[i]->n_operations;
    g_progress[i].completed_ops = 0;
    g_progress[i].thread_id = i % num_threads;
  }

  int active_threads =
      (g_queue_size < num_threads) ? g_queue_size : num_threads;

  for (int i = 0; i < active_threads; i++) {
    thread_data[i].payload_reader = payload_reader;
    thread_data[i].data_offset = data_offset;
    thread_data[i].block_size = manifest->block_size;
    thread_data[i].out_dir = (char *)(uintptr_t)out_dir;
    thread_data[i].reader_mutex = &reader_mutex;
    thread_create(&threads[i], process_partition_thread, &thread_data[i]);
  }

  for (int i = 0; i < active_threads; i++) {
    thread_join(threads[i]);
  }

  free(g_work_queue);
  printf("\nExtraction completed!\n");

  chromeos_update_engine__delta_archive_manifest__free_unpacked(manifest, NULL);
  free(manifest_data);
  reader_cleanup(payload_reader);
  free(payload_reader);
  mutex_destroy(&reader_mutex);
  mutex_destroy(&g_queue_mutex);
  mutex_destroy(&g_progress_mutex);

  return 0;
}

void print_usage(const char *program_name) {
  int term_width = get_terminal_width();
  int option_width = 22;

  printf("Usage: %s <payload_source> [options]\n", program_name);

  for (int i = 0; i < (term_width < 80 ? term_width : 80); i++)
    printf("=");
  printf("\n");

  printf("\nSources:\n");
  printf("  %-*s Local payload.bin or ZIP file\n", option_width, "<file_path>");
#ifdef ENABLE_HTTP_SUPPORT
  printf("  %-*s Remote ZIP file URL\n", option_width, "<http_url>");
#else
  if (term_width > 70) {
    printf("  %-*s Remote ZIP file URL (not available in this build)\n",
           option_width, "<http_url>");
  } else {
    printf("  %-*s Remote ZIP file URL\n", option_width, "<http_url>");
    printf("  %*s (not available in this build)\n", option_width + 2, "");
  }
#endif

  printf("\nOptions:\n");
  printf("  %-*s Output directory (default: output)\n", option_width,
         "--out <dir>");
  printf("  %-*s Comma-separated list of images\n", option_width,
         "--images <list>");
  if (term_width > 70) {
    printf("  %*s to extract\n", option_width + 2, "");
  }
  printf("  %-*s List all partitions and exit\n", option_width, "--list");
  printf("  %-*s Number of threads to use\n", option_width, "--threads <num>");
#ifdef ENABLE_HTTP_SUPPORT
  printf("  %-*s Custom User-Agent for HTTP\n", option_width,
         "--user-agent <ua>");
  if (term_width > 70) {
    printf("  %*s requests\n", option_width + 2, "");
  }
#endif
  printf("  %-*s Show this help message\n", option_width, "--help");

  printf("\n");
  for (int i = 0; i < (term_width < 80 ? term_width : 80); i++)
    printf("=");
  printf("\n");
}

int main(int argc, char *argv[]) {
  const char *user_agent = NULL;
  if (argc < 2) {
    print_usage(argv[0]);
    return -1;
  }

  const char *payload_path = NULL;
  const char *out_dir = "output";
  const char *images_list = "";
  int list_only = 0;
  int num_threads;
#ifdef _WIN32
  SYSTEM_INFO sysinfo;
  GetSystemInfo(&sysinfo);
  num_threads = (int)sysinfo.dwNumberOfProcessors;
#else
  num_threads = (int)sysconf(_SC_NPROCESSORS_ONLN);
#endif
  if (num_threads <= 0 || num_threads > MAX_THREADS) {
    num_threads = 4;
  }

  for (int i = 1; i < argc; i++) {
    if (strcmp(argv[i], "--out") == 0 && i + 1 < argc) {
      out_dir = argv[++i];
    } else if (strcmp(argv[i], "--images") == 0 && i + 1 < argc) {
      images_list = argv[++i];
    } else if (strcmp(argv[i], "--list") == 0) {
      list_only = 1;
    } else if (strcmp(argv[i], "--threads") == 0 && i + 1 < argc) {
      num_threads = atoi(argv[++i]);
      if (num_threads <= 0 || num_threads > MAX_THREADS) {
        num_threads = 4;
      }
    } else if (strcmp(argv[i], "--user-agent") == 0 && i + 1 < argc) {
      user_agent = argv[++i];
    } else if (strcmp(argv[i], "--help") == 0) {
      print_usage(argv[0]);
      return 0;
    } else if (argv[i][0] != '-') {
      if (payload_path == NULL) {
        payload_path = argv[i];
      } else {
        fprintf(stderr, "- Error: Multiple payload paths specified. Only one "
                        "is allowed.\n");
        print_usage(argv[0]);
        return -1;
      }
    } else {
      fprintf(stderr, "- Error: Unknown option '%s'\n", argv[i]);
      print_usage(argv[0]);
      return -1;
    }
  }

  if (payload_path == NULL) {
    fprintf(stderr, "- Error: No payload path/URL specified.\n");
    print_usage(argv[0]);
    return -1;
  }

  printf("- Payload Dumper\n");
  if (!list_only) {
    printf("- Output directory: %s\n", out_dir);
    printf("- Threads: %d\n", num_threads);
    if (strlen(images_list) > 0) {
      printf("- Selected images: %s\n", images_list);
    }
    printf("\n");
  }

  return extract_payload(payload_path, user_agent, out_dir, images_list,
                         list_only, num_threads);
}
