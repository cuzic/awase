// Flutter for Windows(独自の TSF 実装)の TextField 1 つ。lib/main.dart として使う(prepare で flutter create の後に置く)。
import 'package:flutter/material.dart';

void main() => runApp(const MaterialApp(home: Scaffold(body: Padding(padding: EdgeInsets.all(8), child: TextField(autofocus: true, maxLines: 6)))));
