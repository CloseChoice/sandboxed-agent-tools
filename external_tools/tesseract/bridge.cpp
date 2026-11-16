#include <tesseract/baseapi.h>
#include <leptonica/allheaders.h>
#include <cstdint>
#include <cstring>
#include <memory>
#include <cerrno>

#ifdef __wasi__
// Shell helpers are unavailable; never delegate OCR work to a host process.
extern "C" int system(const char *) { errno = ENOSYS; return -1; }
#endif

// Deny any ancillary model/config file access; the model is supplied in memory.
static bool no_file_reader(const char *, std::vector<char> *) { return false; }

extern "C" int sat_ocr(const uint8_t *gray, uint32_t width, uint32_t height,
                        const uint8_t *model, int model_size, const char *language,
                        int psm, int format, char **output) {
    *output = nullptr;
    setLeptDebugOK(0);
    // Image codecs are provided by Rust; hide Leptonica's codec-fallback chatter.
    setMsgSeverity(L_SEVERITY_ERROR);
    tesseract::TessBaseAPI api;
    if (api.Init(reinterpret_cast<const char *>(model), model_size, language,
                 tesseract::OEM_LSTM_ONLY, nullptr, 0, nullptr, nullptr,
                 false, no_file_reader) != 0) return 2;
    api.SetPageSegMode(static_cast<tesseract::PageSegMode>(psm));
    api.SetImage(gray, static_cast<int>(width), static_cast<int>(height), 1,
                 static_cast<int>(width));
    api.SetSourceResolution(300);
    if (api.Recognize(nullptr) != 0) return 3;
    if (format == 1) *output = api.GetTSVText(0);
    else if (format == 2) *output = api.GetHOCRText(0);
    else *output = api.GetUTF8Text();
    return *output == nullptr ? 4 : 0;
}

extern "C" void sat_ocr_free(char *output) { delete[] output; }
