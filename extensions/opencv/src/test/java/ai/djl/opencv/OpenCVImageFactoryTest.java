/*
 * Copyright 2021 Amazon.com, Inc. or its affiliates. All Rights Reserved.
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
package ai.djl.opencv;

import ai.djl.modality.cv.BufferedImageFactory;
import ai.djl.modality.cv.Image;
import ai.djl.modality.cv.ImageFactory;
import ai.djl.modality.cv.output.BoundingBox;
import ai.djl.modality.cv.output.DetectedObjects;
import ai.djl.modality.cv.output.Joints;
import ai.djl.modality.cv.output.Landmark;
import ai.djl.modality.cv.output.Mask;
import ai.djl.modality.cv.output.Rectangle;
import ai.djl.ndarray.NDArray;
import ai.djl.ndarray.NDManager;
import ai.djl.ndarray.types.DataType;
import ai.djl.testing.TestRequirements;
import java.io.IOException;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import org.testng.Assert;
import org.testng.annotations.Test;

public class OpenCVImageFactoryTest {

    @Test
    public void testImage() throws IOException {
        TestRequirements.notWindows(); // failed on Windows ServerCore container
        TestRequirements.notArm();

        ImageFactory factory = ImageFactory.getInstance();
        ImageFactory defFactory = new BufferedImageFactory();
        Path path = Paths.get("../../examples/src/test/resources/kitten.jpg");
        String url = path.toUri().toURL().toString();
        Image gold = defFactory.fromFile(path);

        try (NDManager manager = NDManager.newBaseManager()) {
            NDArray expected = gold.toNDArray(manager);

            Image img = factory.fromFile(path);
            Assert.assertEquals(img.getWidth(), gold.getWidth());
            Assert.assertEquals(img.getHeight(), gold.getHeight());
            Assert.assertEquals(img.getWrappedImage().getClass().getSimpleName(), "Mat");

            NDArray array = img.toNDArray(manager);
            Assert.assertEquals(array, expected);

            Image newImage = factory.fromNDArray(array);
            Assert.assertEquals(newImage.getWidth(), gold.getWidth());
            Assert.assertEquals(newImage.getHeight(), gold.getHeight());
            array = array.transpose(2, 0, 1);
            newImage = factory.fromNDArray(array);
            Assert.assertEquals(newImage.getWidth(), gold.getWidth());
            Assert.assertEquals(newImage.getHeight(), gold.getHeight());

            img = factory.fromUrl(url);
            array = img.toNDArray(manager);
            Assert.assertEquals(array, expected);

            newImage = factory.fromImage(img.getWrappedImage());
            newImage = newImage.getSubImage(0, 0, 4, 2);

            array = newImage.toNDArray(manager, Image.Flag.GRAYSCALE);
            expected = gold.getSubImage(0, 0, 4, 2).toNDArray(manager, Image.Flag.GRAYSCALE);
            Assert.assertEquals(array, expected);

            List<String> list = Collections.singletonList("cat");
            List<Double> prob = Collections.singletonList(0.9);
            List<ai.djl.modality.cv.output.Point> points = new ArrayList<>();
            points.add(new ai.djl.modality.cv.output.Point(120, 160));
            List<BoundingBox> boundingBoxes =
                    Collections.singletonList(new Landmark(0.35, 0.15, 0.5, 0.6, points));

            DetectedObjects detectedObjects = new DetectedObjects(list, prob, boundingBoxes);
            Image imgCopy = img.duplicate();
            imgCopy.drawBoundingBoxes(detectedObjects);

            float[][] maskProb = {
                {1f, 1f, 1f, 1f, 1f, 1f, 1f},
                {1f, 1f, 1f, 1f, 1f, 1f, 1f},
                {1f, 1f, 1f, 1f, 1f, 1f, 1f},
                {1f, 1f, 1f, 1f, 1f, 1f, 1f},
                {1f, 1f, 1f, 1f, 1f, 1f, 1f},
                {1f, 1f, 1f, 1f, 1f, 1f, 1f}
            };
            List<BoundingBox> masks =
                    Collections.singletonList(new Mask(0.1, 0.5, 0.1, 0.1, maskProb));

            DetectedObjects mask = new DetectedObjects(list, prob, masks);
            imgCopy.drawBoundingBoxes(mask);

            List<Joints.Joint> jointList =
                    Collections.singletonList(new Joints.Joint(0.2, 0.2, 0.8));
            Joints joints = new Joints(jointList);
            imgCopy.drawJoints(joints);

            try (OutputStream os = Files.newOutputStream(Paths.get("build/newImage.png"))) {
                imgCopy.save(os, "png");
            }

            Assert.assertThrows(
                    IOException.class,
                    () -> {
                        factory.fromFile(Paths.get("nonexist.jpg"));
                    });
            Assert.assertThrows(
                    IOException.class,
                    () -> {
                        factory.fromUrl("file:build.gradle");
                    });
        }
    }

    @Test
    public void testBoundingBoxes() {
        TestRequirements.notWindows(); // failed on Windows ServerCore container
        TestRequirements.notArm();

        ImageFactory factory = ImageFactory.getInstance();
        try (NDManager manager = NDManager.newBaseManager()) {
            int[][] arr =
                    new int[][] {
                        {0, 1, 1, 1, 0},
                        {0, 0, 1, 0, 0},
                        {0, 0, 0, 0, 0},
                        {1, 1, 0, 0, 0},
                        {1, 0, 0, 0, 1}
                    };
            NDArray array = manager.create(arr).muli(255).toType(DataType.INT8, true).expandDims(0);
            OpenCVImage image = (OpenCVImage) factory.fromNDArray(array);
            List<BoundingBox> rectangles = image.findBoundingBoxes();
            List<Rectangle> expected =
                    Arrays.asList(
                            new Rectangle(0.8, 0.8, 0.2, 0.2),
                            new Rectangle(0, 0.6, 0.4, 0.4),
                            new Rectangle(0.2, 0, 0.6, 0.4));
            for (int i = 0; i < rectangles.size(); i++) {
                Assert.assertEquals(rectangles.get(i).toString(), expected.get(i).toString());
            }
        }
    }
}
