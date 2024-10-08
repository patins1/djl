/*
 * Copyright 2019 Amazon.com, Inc. or its affiliates. All Rights Reserved.
 *
 * Licensed under the Apache License, Version 2.0 (the "License"). You may not use this file except in compliance
 * with the License. A copy of the License is located at
 *
 * http://aws.amazon.com/apache2.0/
 *
 * or in the "license" file accompanying this file. This file is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES
 * OR CONDITIONS OF ANY KIND, either express or implied. See the License for the specific language governing permissions
 * and limitations under the License.
 */
package ai.djl.examples.inference.cv;

import ai.djl.ModelException;
import ai.djl.inference.Predictor;
import ai.djl.modality.Classifications;
import ai.djl.repository.zoo.Criteria;
import ai.djl.repository.zoo.ZooModel;
import ai.djl.training.util.ProgressBar;
import ai.djl.translate.TranslateException;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.IOException;
import java.net.URL;

/**
 * An example of inference using an action recognition model.
 *
 * <p>See this <a
 * href="https://github.com/deepjavalibrary/djl/blob/master/examples/docs/action_recognition.md">doc</a>
 * for information about this example.
 */
public final class ActionRecognition {

    private static final Logger logger = LoggerFactory.getLogger(ActionRecognition.class);

    private ActionRecognition() {}

    public static void main(String[] args) throws IOException, ModelException, TranslateException {
        Classifications classification = predict();
        logger.info("{}", classification);
    }

    public static Classifications predict() throws IOException, ModelException, TranslateException {
        URL url = new URL("https://resources.djl.ai/images/action_dance.jpg");
        // Use DJL PyTorch model zoo model
        Criteria<URL, Classifications> criteria =
                Criteria.builder()
                        .setTypes(URL.class, Classifications.class)
                        .optModelUrls(
                                "djl://ai.djl.pytorch/Human-Action-Recognition-VIT-Base-patch16-224")
                        .optEngine("PyTorch")
                        .optProgress(new ProgressBar())
                        .build();

        try (ZooModel<URL, Classifications> inception = criteria.loadModel();
                Predictor<URL, Classifications> action = inception.newPredictor()) {
            return action.predict(url);
        }
    }
}
