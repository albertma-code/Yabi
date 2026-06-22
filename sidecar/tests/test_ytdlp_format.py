import unittest

from bilio_sidecar.ytdlp import _best_video_format, _format_label, _format_with_audio


class FormatWithAudioTest(unittest.TestCase):
    def test_exact_video_format_id_gets_best_audio(self):
        self.assertEqual(_format_with_audio("30080"), "30080+bestaudio/best")

    def test_complete_format_expression_is_preserved(self):
        self.assertEqual(
            _format_with_audio("bv*[height<=1080]+ba/best"),
            "bv*[height<=1080]+ba/best",
        )

    def test_fallback_expression_is_preserved(self):
        self.assertEqual(_format_with_audio("bestvideo/best"), "bestvideo/best")

    def test_empty_format_stays_empty(self):
        self.assertEqual(_format_with_audio("   "), "")


class BestVideoFormatTest(unittest.TestCase):
    def test_picks_highest_quality_video_and_ignores_audio(self):
        best = _best_video_format([
            {"format_id": "30280", "vcodec": "none", "height": None, "quality": None},
            {"format_id": "30032", "vcodec": "avc1", "height": 480, "width": 854, "quality": 32},
            {"format_id": "30080", "vcodec": "avc1", "height": 1080, "width": 1920, "quality": 80},
        ])
        self.assertIsNotNone(best)
        self.assertEqual(best["format_id"], "30080")

    def test_format_label_prefers_bilibili_format_name(self):
        self.assertEqual(
            _format_label({"format": "1080P 高清", "format_note": None, "height": 1080}),
            "1080P 高清",
        )


if __name__ == "__main__":
    unittest.main()
